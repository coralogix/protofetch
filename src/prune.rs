use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PruneError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
}

pub fn extract_dependencies(file : &Path) -> Result<Vec<String>, PruneError> {
    let mut dependencies = Vec::new();
    let mut reader = BufReader::new(File::open(file)?);
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        if line.starts_with("import ") {
            if let Some(dependency) = line.split_whitespace().nth(1) {
                let dependency = dependency.to_string().replace(';', "").replace('\"', "");
                dependencies.push(dependency.to_string());
            }
        }
        line.clear();
    }
    Ok(dependencies)
}

#[test]
fn extract_dependencies_test() {
    let path = project_root::get_project_root().unwrap().join(Path::new("resources/example.proto"));
    let dependencies = extract_dependencies(&path).unwrap();
    assert_eq!(dependencies.len(), 3);
    assert_eq!(dependencies[0], "scalapb/scalapb.proto");
    assert_eq!(dependencies[1], "google/protobuf/descriptor.proto");
    assert_eq!(dependencies[2], "google/protobuf/struct.proto");
}