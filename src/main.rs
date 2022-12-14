use anyhow::{bail, Result};
use bollard::Docker;
use futures_util::{future::ready, StreamExt};
use log::{debug, error};
use serde::Deserialize;
use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Write},
    os::unix::prelude::PermissionsExt,
    path::{Path, PathBuf},
};
use structopt::StructOpt;
use tar::Archive;

#[derive(Debug, StructOpt)]
#[structopt(
    name = "dext",
    about = "Extracts a docker image's layers to a specified location.",
    author = env!("CARGO_PKG_AUTHORS"),
)]
struct Opts {
    /// Docker image name
    #[structopt(short = "i", long = "image")]
    image_name: Option<String>,

    /// Docker image version
    // If not specified will default to 'latest'.
    #[structopt(short = "v", long = "version", default_value = "latest")]
    image_version: String,

    /// Image archive file (.tar)
    #[structopt(short = "f", long = "file")]
    image_file: Option<PathBuf>,

    /// Output folder
    // Location that must be a folder to write all of the image layers.
    #[structopt(parse(from_os_str))]
    out_path: PathBuf,

    /// Write entrypoint?
    // Writes the entrypoint from the image to a file.\
    #[structopt(short = "e", long = "entrypoint")]
    write_entrypoint: bool,

    /// Entrypoint file name, relative to out_path.
    #[structopt(long = "entry-file", default_value = "entrypoint.sh")]
    entrypoint: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let opts = Opts::from_args();

    if !opts.out_path.is_dir() {
        bail!(
            "location specified is not a directory: {}",
            opts.out_path.to_string_lossy()
        );
    }

    let tmp = tempdir::TempDir::new("image-builder")?;
    debug!("temp dir: {}", tmp.path().to_string_lossy());
    let tar_path = {
        match (opts.image_name, opts.image_file) {
            (Some(image_name), None) => {
                if image_name.contains(":") {
                    bail!("image name should be the name only - use the --version flag to specify a version.");
                }

                let image = format!("{}:{}", image_name, opts.image_version);

                fetch_archive(tmp.path(), &image).await?
            }
            (None, Some(tar_path)) => tar_path,
            (Some(_), Some(_)) => {
                error!("cannot specify both image name and image file.");
                bail!("you cannot specify both an image name as well as an image file.")
            }
            (None, None) => {
                error!("not enough arguments specified.");
                bail!("you must specify either an image name or an archive path.")
            }
        }
    };
    let manifest = extract_layers(&tar_path, &opts.out_path, tmp.path()).await?;

    if opts.write_entrypoint {
        write_entrypoint(&manifest, tmp.path(), &opts.out_path, opts.entrypoint)?;
    }

    Ok(())
}

async fn fetch_archive(tmp: &Path, image: &str) -> Result<PathBuf> {
    let tar_name = format!("{image}.tar");
    let mut tar_path = PathBuf::new();
    tar_path.push(&tmp);
    tar_path.push(&tar_name);
    debug!("tar file: {}", tar_path.to_string_lossy());

    let docker = Docker::connect_with_local_defaults()?;
    // Make sure the image is there.
    docker.inspect_image(image).await?;

    let byte_stream = docker.export_image(image);

    debug!("exporting image: {image}");
    let mut writer = BufWriter::new(File::create(&tar_path)?);
    byte_stream
        .for_each(move |data| {
            writer
                .write_all(&*data.expect("error streaming data from docker"))
                .expect("error writing file to disk");
            writer.flush().expect("could not flush");
            ready(())
        })
        .await;

    Ok(tar_path)
}

async fn extract_layers(tar_path: &Path, out_path: &Path, tmp: &Path) -> Result<Manifest> {
    let reader = BufReader::new(File::open(tar_path)?);
    let mut archive = Archive::new(reader);
    debug!("unpacking archive: {}", tar_path.to_string_lossy());
    archive.unpack(&tmp)?;

    fs::remove_file(&tar_path)?;

    let mut mf_path = PathBuf::new();
    mf_path.push(&tmp);
    mf_path.push("manifest.json");

    let manifest = read_manifest(&File::open(mf_path)?)?;
    debug!("read manifest and found {} layers", manifest.layers.len());

    for layer in manifest.layers.iter() {
        let mut layer_path = PathBuf::new();
        layer_path.push(&tmp);
        layer_path.push(layer);
        let reader = BufReader::new(File::open(&layer_path)?);
        let mut archive = Archive::new(reader);
        debug!("unpacking layer: {}", layer_path.to_string_lossy());
        archive.unpack(out_path)?;
    }

    Ok(manifest)
}

#[derive(Deserialize, Debug)]
struct Manifest {
    #[serde(alias = "Config")]
    config: String,
    // #[serde(alias = "RepoTags")]
    // repo_tags: Vec<String>,
    #[serde(alias = "Layers")]
    layers: Vec<String>,
}

fn read_manifest(manifest: &File) -> Result<Manifest> {
    let mf = BufReader::new(manifest);
    let manifest = {
        let mut manifests: Vec<Manifest> = serde_json::from_reader(mf)?;

        if manifests.len() != 1 {
            // We should only ever get the manifest for a single version
            bail!(
                "the manifest contains {} entries, expected 1",
                manifests.len()
            );
        }
        manifests.pop().expect("we just checked the length")
    };

    Ok(manifest)
}

#[derive(Deserialize, Debug)]

struct ImageConfig {
    config: Config,
}

#[derive(Deserialize, Debug)]

struct Config {
    #[serde(alias = "Env")]
    env: Vec<String>,
    #[serde(alias = "Cmd")]
    cmd: Vec<String>,
    #[serde(alias = "WorkingDir")]
    working_dir: String,
}

fn read_config(config: &File) -> Result<ImageConfig> {
    let config = BufReader::new(config);
    Ok(serde_json::from_reader(config)?)
}

fn write_entrypoint(
    manifest: &Manifest,
    tmp: &Path,
    out_path: &Path,
    entrypoint: String,
) -> Result<()> {
    let mut cfg = PathBuf::new();
    cfg.push(&tmp);
    cfg.push(&manifest.config);
    debug!(
        "reading image configuration from: {}",
        cfg.to_string_lossy()
    );
    let config = read_config(&File::open(cfg)?)?;

    let mut ep_file = PathBuf::new();
    ep_file.push(&out_path);
    ep_file.push(entrypoint);
    debug!("writing entrypoint file to: {}", ep_file.to_string_lossy());

    let mut w = BufWriter::new(File::create(&ep_file)?);

    writeln!(w, "#!/bin/bash")?;
    for env in config.config.env.iter() {
        writeln!(w, "{env}")?;
    }
    writeln!(w, "cd {}", config.config.working_dir)?;
    for cmd in config.config.cmd.iter() {
        writeln!(w, "{cmd}")?;
    }

    fs::set_permissions(&ep_file, fs::Permissions::from_mode(0o755))?;

    Ok(())
}
