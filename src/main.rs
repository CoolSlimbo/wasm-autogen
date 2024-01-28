mod config;
mod decleration_functions;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use clap::Parser;
use config::*;
use confique::{toml, Config};
use decleration_functions::*;
use path_absolutize::*;
use proc_macro2::TokenStream;
use quote::quote;
use std::{io::IsTerminal, path::PathBuf};
use swc_common::sync::Lrc;
use swc_common::{
    errors::{ColorConfig, Handler},
    SourceMap,
};
use swc_ecma_ast::{Decl, Module, ModuleDecl, ModuleItem, Stmt};
use swc_ecma_parser::{lexer::Lexer, Parser as JSParser, StringInput, Syntax};
use tracing_subscriber::prelude::*;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Autogen {
    // To be verbrose, or not to be vebrose
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    // The custom input file to use
    #[arg(short, long)]
    pub input: Option<PathBuf>,
    // Regenerate the config file
    #[arg(short, long)]
    pub regenerate: bool,
}

fn main() -> Result<()> {
    let autogen = Autogen::parse();

    // std::env::set_var("RUST_BACKTRACE", "1");

    // Config logging
    // Taken from trunk.rs
    {
        let colored = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();

        #[cfg(windows)]
        if colored {
            if let Err(err) = ansi_term::enable_ansi_support() {
                eprintln!("error enabling ANSI support: {:?}", err);
            }
        }

        tracing_subscriber::registry()
            .with(eval_logging(&autogen))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(colored)
                    .with_target(false)
                    .with_level(true)
                    .compact(),
            )
            .try_init()
            .context("failed to initialize logging")?;
    }

    tracing::info!("Starting autogen.");
    tracing::debug!("CLI settings: {:?}", autogen);

    let config_path = autogen
        .input
        .unwrap_or_else(|| PathBuf::from("autogen.toml"));

    match autogen.regenerate {
        true => {
            tracing::info!("Regenerating config file.");
            generate_config(config_path.clone())?;
            tracing::info!("Config file regenerated.");
        }
        false => match config_path.exists() {
            true => {
                tracing::info!("Config file found.");
                tracing::info!("Loading config from {:?}", config_path);
            }
            false => {
                tracing::warn!("Config file not found. Generating default config.");
                generate_config(config_path.clone())?;
                tracing::info!("Config file generated at autogen.toml.");
            }
        },
    }

    let config = GenerateConfig::from_file(config_path).context("Failed to load config file.")?;
    tracing::info!("Config loaded.");
    tracing::debug!("Config: {:?}", config);

    tracing::info!("Mapping files...");
    let mut mappings =
        map_files(&config).context("Error source mapping out typescript imports.")?;
    tracing::info!("Files mapped.");

    tracing::info!("Mapping statements");
    let mapped_statements =
        map_statements(&mut mappings).context("Error mapping out typescript statements.")?;
    tracing::info!("Statements mapped.");

    // Mapped_statements is an array of PathBufs to TokenStreams
    // We need to map them out to the correct files.
    save_statements(&mapped_statements, &config).context("Failed to save statements.")?;

    Ok(())
}

type FileMapping = Vec<PathBuf>;
type MappedStatements = Vec<(PathBuf, TokenStream)>;

fn save_statements(mappings: &MappedStatements, config: &GenerateConfig) -> Result<()> {
    let output = config.output.directory.clone();
    let output = output
        .absolutize()
        .context("Failed to canonicalize path.")?
        .to_path_buf();

    // To get the names and folders right, we need to isolate the PathBuf's down to whatever the dir before the input file is
    let derviation = config.input.index_file.clone();
    let mut derviation = derviation
        .absolutize()
        .context("Failed to canonicalize path.")?
        .to_path_buf();
    derviation.pop();

    // tracing::debug!("Derviation: {:?}", derviation);

    // Strip the derviation from the mappings
    let mappings = mappings
        .iter()
        .map(|(file, stream)| {
            let mut file = file.clone();
            file = file
                .absolutize()
                .context("Failed to canonicalize path.")?
                .to_path_buf();
            file = file
                .strip_prefix(&derviation)
                .context("Failed to strip derviation from path.")?
                .to_path_buf();
            // Add the output directory to the path
            let mut file = output.join(file);
            // Change the extension to .rs
            file.set_extension("rs");
            Ok((file, stream))
        })
        .collect::<Result<Vec<_>>>()?;

    tracing::debug!(
        "Mappings: {:#?}",
        mappings.iter().map(|(file, _)| file).collect::<Vec<_>>()
    );

    // Quickly clear the output directory
    let _ = std::fs::remove_dir_all(&output).context("Failed to clear output directory.");

    for (file, stream) in mappings {
        let name = file
            .file_name()
            .context("Failed to get file name.")?
            .to_str()
            .context("Failed to convert file name to string.")?
            .to_string();
        let mut file = file.clone();
        file.pop();
        std::fs::create_dir_all(&file).context("Failed to create output directory.")?;

        let mut file = file.clone();
        file.push(name);

        tracing::debug!("Writing file: {:?}", file);

        let mut file = std::fs::File::create(file).context("Failed to create output file.")?;

        let stream = quote! {
            #stream
        };

        let parsed = syn::parse2(stream).context("Unable to parse token stream.")?;
        let parsed = prettyplease::unparse(&parsed);
        let parsed = parsed.replace("\\n", "\n");

        tracing::trace!("Output file: {:#?}", parsed);

        std::io::Write::write_all(&mut file, parsed.as_bytes())
            .context("Failed to write output file.")?;
    }

    Ok(())
}

fn map_statements(mappings: &mut FileMapping) -> Result<MappedStatements> {
    let mut mapped_streams = Vec::new();
    for file in mappings {
        let module = parse_file(file).expect("Failed to parse file.");

        // Ignore any ModuleDecl item
        let statements = module
            .body
            .iter()
            .filter_map(|item| match item {
                ModuleItem::Stmt(stmt) => match stmt {
                    Stmt::Decl(stmt) => Some(stmt),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();

        tracing::trace!("Statements from file {:?}: {:#?}", file, statements);
        tracing::debug!(
            "Mapping {} statements from file: {:?}",
            statements.len(),
            file.absolutize()
                .context("Failed to canonicalize path.")?
                .display()
        );

        let mut streams = Vec::<TokenStream>::new();

        for statement in statements {
            let stream = match statement {
                Decl::Class(class) => map_class(class),
                _ => Ok(None),
            }
            .context("Failed to map statement.")?;

            let stream = match stream {
                Some(stream) => stream,
                None => continue,
            };

            streams.push(stream);
        }

        tracing::trace!("Streams: {:#?}", streams);

        // For now, we'll write the file to the log
        let outstream = quote! {
            #[wasm_bindgen]
            extern "C" {
                #(#streams)*
            }
        };

        // tracing::trace!("Output file: {:#?}", outstream);

        // let parsed = syn::parse2(outstream).context("Unable to parse token stream.")?;
        // let parsed = prettyplease::unparse(&parsed);
        // let parsed = parsed.replace("\\n", "\n");
        // tracing::trace!("Output file: {:#?}", parsed);

        mapped_streams.push((file.clone(), outstream));
    }

    Ok(mapped_streams)
}

fn map_files(config: &GenerateConfig) -> Result<FileMapping> {
    fn map_file(file: &mut PathBuf, exsisting: &mut FileMapping) -> Result<FileMapping> {
        if exsisting.contains(&file) {
            tracing::trace!("File already source mapped. Ignoring.");
            return Ok(Vec::new());
        }

        let module = parse_file(file)?;

        let imports = module
            .body
            .iter()
            .filter_map(|item| match item {
                ModuleItem::ModuleDecl(decl) => match decl {
                    ModuleDecl::Import(import) => Some(import),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();

        // We also have to handle when a file exports all, via the asterisk, as we need to then take those as if they were imports
        let exports = module
            .body
            .iter()
            .filter_map(|item| match item {
                ModuleItem::ModuleDecl(decl) => match decl {
                    ModuleDecl::ExportAll(export) => Some(export),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();

        tracing::trace!("Imports: {:#?}", imports);
        tracing::trace!("Importing exports: {:#?}", exports);

        tracing::trace!(
            "Imported file: {}",
            file.absolutize()
                .context("Failed to canonicalize path.")?
                .display()
        );

        let mut mappings = Vec::new();

        mappings.push(file.clone());
        exsisting.push(file.clone());

        for (i, import) in imports.iter().enumerate() {
            // import.src.value is the file path
            let mut file_path = file.clone();
            file_path.pop();
            file_path.push(&import.src.value.as_str());
            // The file path doesn't have an extension, so we add .ts
            file_path.set_extension("ts");
            let mut file_path = file_path
                .absolutize()
                .context("Failed to canonicalize path.")?
                .to_path_buf();

            tracing::trace!("Import {}: {:#?}", i, import);
            tracing::trace!("Importing file path: {:?}", &file_path);

            let mut mapping = map_file(&mut file_path, &mut exsisting.as_mut())
                .context(format!("Error source mapping file: {:#?}", &file_path))?;
            mappings.append(&mut mapping);
        }

        for (i, export) in exports.iter().enumerate() {
            // export.src.value is the file path
            let mut file_path = file.clone();
            file_path.pop();
            file_path.push(&export.src.value.as_str());
            // The file path doesn't have an extension, so we add .ts
            file_path.set_extension("ts");
            let mut file_path = file_path
                .absolutize()
                .context("Failed to canonicalize path.")?
                .to_path_buf();

            tracing::trace!("Export import {}: {:#?}", i, export);
            tracing::trace!("Export imorting file path: {:?}", &file_path);

            let mut mapping = map_file(&mut file_path, &mut exsisting.as_mut())
                .context(format!("Error source mapping file: {:#?}", &file_path))?;
            mappings.append(&mut mapping);
        }

        // tracing::trace!("Parsed module: {:?}", module);

        Ok(mappings)
    }
    // We use the map_file function recursivley to map all files from the base file
    // This is done so that we can map all files in the correct order
    let mut mappings = Vec::new();
    let mut mapping = map_file(&mut config.input.index_file.clone(), &mut mappings)
        .context("Failed to source map input file.")?;

    mapping.sort();
    mapping.dedup();

    for file in &mapping {
        tracing::debug!(
            "Source mapped file: {:?}",
            file.absolutize().context("Failed to canonicalize path.")?
        );
    }
    tracing::debug!("Total files mapped: {}", mapping.len());

    Ok(mapping)
}

fn parse_file(file: &mut PathBuf) -> Result<Module> {
    let file = file
        .canonicalize()
        .context("Failed to canonicalize path.")?;
    let cm = Lrc::<SourceMap>::default();
    let handler = Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));

    let fm = cm.load_file(&file).context("Failed to load input file.")?;

    let lexer = Lexer::new(
        Syntax::Typescript(Default::default()),
        Default::default(),
        StringInput::from(&*fm),
        None,
    );

    let mut parser = JSParser::new_from(lexer);

    for e in parser.take_errors() {
        e.into_diagnostic(&handler).emit();
    }

    Ok(parser
        .parse_typescript_module()
        .map_err(|e| e.into_diagnostic(&handler).emit())
        .expect("Failed to parse input file."))
}

fn generate_config(location: PathBuf) -> Result<()> {
    let template_string = toml::template::<GenerateConfig>(toml::FormatOptions::default());
    std::fs::write(location, template_string).context("Failed to write config file.")
}

// Function coutesy of trunk.rs
fn eval_logging(cli: &Autogen) -> tracing_subscriber::EnvFilter {
    let directives = match cli.verbose {
        2 => {
            tracing::info!("Extra verbose logging enabled.");
            "error,wasm_autogen=trace"
        }
        1 => {
            tracing::info!("Slightly verbose logging enabled.");
            "error,wasm_autogen=debug"
        }
        _ => "error,wasm_autogen=info",
    };

    tracing_subscriber::EnvFilter::new(directives)
}
