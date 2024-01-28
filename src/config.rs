use confique::Config;
use std::path::PathBuf;

#[derive(Config, Debug)]
pub struct GenerateConfig {
    #[config(nested)]
    pub input: Input,
    #[config(nested)]
    pub output: Output,
}

#[derive(Config, Debug)]
pub struct Input {
    // The index.ts file to use as the input.
    // Everything is resolved relative to this file.
    #[config(default = "ts/index.ts")]
    pub index_file: PathBuf,
}

#[derive(Config, Debug)]
pub struct Output {
    // The output directory to use.
    #[config(default = "output")]
    pub directory: PathBuf,
}
