# `docker_extract`

A CLI tool to extract the layers of a local `docker` image and overlay the contents of them into a specified folder.

```
Extracts a docker image's layers to a specified location.

USAGE:
    docker-extract [FLAGS] [OPTIONS] <image-name> <out-path>

FLAGS:
    -h, --help          Prints help information
    -e, --entrypoint    Write entrypoint?

OPTIONS:
    -f, --entry-file <entrypoint>    Entrypoint file name, relative to out_path [default: entrypoint.sh]
    -v, --version <image-version>    Docker image version [default: latest]

ARGS:
    <image-name>    Docker image name
    <out-path>      Output folder
```

## Installation

```
$ cargo install docker_extract
```

## Usage

To write out the contents of the `rust` image:

```
$ docker pull rust

...

$ mkdir rust_image
$ docker_extract rust rust_image
```

To write the contents of an image (`image`), including a script to invoke it:

```
$ mkdir my_image
$ docker_extract -e image my_image
```

This will create the file `my_image/entrypoint.sh` that when invoked with `my_image` as root (e.g. in a VM or chroot environment) will invoke the image entrypoint.

## Logging

Debug logging can be enabled with the `RUST_LOG` environment variable.

```
$ RUST_LOG=docker_extract=debug docker_extract ...
```
