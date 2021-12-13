use anyhow::{Context, Result};
use std::{collections::HashMap, fs::File, io::Read, path::PathBuf};
use structopt::StructOpt;
use walkdir::WalkDir;

/// Measure the available wins from deduplicating various parts of WebAssembly
/// binaries.
///
/// ### Example
///
/// $ measure-wasm-dedupe-wins path/to/corpus/of/Wasm/binaries
#[derive(StructOpt)]
struct Options {
    /// A directory containing the Wasm binaries we should measure.
    ///
    /// This directory tree is recursively traversed to find Wasm binaries.
    #[structopt(parse(from_os_str))]
    corpus: PathBuf,
}

fn main() -> Result<()> {
    env_logger::init();

    let options = Options::from_args();
    let mut counts = Counts::default();
    let mut wasm = vec![];

    for entry in WalkDir::new(&options.corpus).follow_links(true) {
        let entry = entry.context("failed to read directory entry")?;

        // Only consider `.wasm` paths.
        if !entry.path().extension().map_or(false, |ext| ext == "wasm") {
            log::debug!("Ignoring non-Wasm entry: {}", entry.path().display());
            continue;
        }

        // Only consider files.
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to read metadata of {}", entry.path().display()))?;
        if !metadata.is_file() {
            log::debug!("Ignoring non-file entry: {}", entry.path().display());
            continue;
        }

        log::info!("Considering Wasm file: {}", entry.path().display());

        let mut file = File::open(entry.path())
            .with_context(|| format!("failed to open {}", entry.path().display()))?;

        wasm.clear();
        file.read_to_end(&mut wasm)
            .with_context(|| format!("failed to read {}", entry.path().display()))?;

        counts
            .add_wasm(&wasm)
            .with_context(|| format!("failed to count {}", entry.path().display()))?;
    }

    println!("Total size:                 {:>9} bytes", counts.total_size);

    println!("--------------------------------------------------------------------------------");

    let dupe_data = counts.duplicated_data_segments();
    println!(
        "Duplicated data segments:   {:>9} bytes ({:.02}%)",
        dupe_data,
        dupe_data as f64 / counts.total_size as f64 * 100.0
    );

    let dupe_elem = counts.duplicated_elem_segments();
    println!(
        "Duplicated elem segments:   {:>9} bytes ({:.02}%)",
        dupe_elem,
        dupe_elem as f64 / counts.total_size as f64 * 100.0
    );

    let dupe_code = counts.duplicated_code_bodies();
    println!(
        "Duplicated code bodies:     {:>9} bytes ({:.02}%)",
        dupe_code,
        dupe_code as f64 / counts.total_size as f64 * 100.0
    );

    let dupe_custom = counts.duplicated_custom_sections();
    println!(
        "Duplicated custom sections: {:>9} bytes ({:.02}%)",
        dupe_custom,
        dupe_custom as f64 / counts.total_size as f64 * 100.0
    );

    println!("--------------------------------------------------------------------------------");

    let dupe_total = dupe_data + dupe_elem + dupe_code + dupe_custom;
    println!(
        "Total duplicated data:      {:>9} bytes ({:.02}%)",
        dupe_total,
        dupe_total as f64 / counts.total_size as f64 * 100.0
    );

    Ok(())
}

type WideHash = [u8; 512];

fn hash(data: &[u8]) -> WideHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(data);
    let mut reader = hasher.finalize_xof();
    let mut hash = [0; 512];
    reader.fill(&mut hash);
    hash
}

struct SizeEntry {
    size: u64,
    count: u64,
}

#[derive(Default)]
struct Counts {
    total_size: u64,
    data_segments: HashMap<WideHash, SizeEntry>,
    elem_segments: HashMap<WideHash, SizeEntry>,
    code_bodies: HashMap<WideHash, SizeEntry>,
    custom_sections: HashMap<WideHash, SizeEntry>,
}

impl Counts {
    fn add_entry(map: &mut HashMap<WideHash, SizeEntry>, data: &[u8]) {
        let hash = hash(data);
        map.entry(hash)
            .or_insert_with(|| SizeEntry {
                size: data.len() as u64,
                count: 0,
            })
            .count += 1;
    }

    fn add_data_segment(&mut self, data_segment: &[u8]) {
        Self::add_entry(&mut self.data_segments, data_segment);
    }

    fn add_elem_segment(&mut self, elem_segment: &[u8]) {
        Self::add_entry(&mut self.elem_segments, elem_segment);
    }

    fn add_code_body(&mut self, code_body: &[u8]) {
        Self::add_entry(&mut self.code_bodies, code_body);
    }

    fn add_custom_section(&mut self, custom: &[u8]) {
        Self::add_entry(&mut self.custom_sections, custom);
    }

    fn add_wasm(&mut self, full_wasm: &[u8]) -> Result<()> {
        self.total_size += full_wasm.len() as u64;

        let mut input = full_wasm;
        let mut parsers = vec![wasmparser::Parser::new(0)];
        while !parsers.is_empty() {
            let (payload, consumed) = match parsers
                .last_mut()
                .unwrap()
                .parse(input, true)
                .context("failed to parse Wasm")?
            {
                wasmparser::Chunk::NeedMoreData(_) => unreachable!(),
                wasmparser::Chunk::Parsed { consumed, payload } => (payload, consumed),
            };
            input = &input[consumed..];

            match payload {
                wasmparser::Payload::DataSection(mut reader) => {
                    for _ in 0..reader.get_count() {
                        let data = reader.read()?;
                        self.add_data_segment(&full_wasm[data.range.start..data.range.end]);
                    }
                }
                wasmparser::Payload::ElementSection(mut reader) => {
                    for _ in 0..reader.get_count() {
                        let elem = reader.read()?;
                        self.add_elem_segment(&full_wasm[elem.range.start..elem.range.end]);
                    }
                }
                wasmparser::Payload::CodeSectionEntry(body) => {
                    self.add_code_body(&full_wasm[body.range().start..body.range().end]);
                }
                wasmparser::Payload::CustomSection { data, .. } => {
                    self.add_custom_section(data);
                }
                wasmparser::Payload::ModuleSectionEntry { parser, .. } => {
                    parsers.push(parser);
                }
                wasmparser::Payload::End => {
                    parsers.pop();
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn duplicated_data_segments(&self) -> u64 {
        self.data_segments
            .values()
            .map(|entry| entry.size * (entry.count - 1))
            .sum()
    }

    fn duplicated_elem_segments(&self) -> u64 {
        self.elem_segments
            .values()
            .map(|entry| entry.size * (entry.count - 1))
            .sum()
    }

    fn duplicated_code_bodies(&self) -> u64 {
        self.code_bodies
            .values()
            .map(|entry| entry.size * (entry.count - 1))
            .sum()
    }

    fn duplicated_custom_sections(&self) -> u64 {
        self.custom_sections
            .values()
            .map(|entry| entry.size * (entry.count - 1))
            .sum()
    }
}
