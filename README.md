# Eva

This repo contains the implementation of the paper "Eva: Efficient Privacy-Preserving Proof of Authenticity for Lossily Encoded Videos" (IEEE S&P 2025). The dataset can be found on [Hugging Face](https://huggingface.co/datasets/winderica/eva-dataset).

The implementation is based on [Sonobe](https://github.com/privacy-scaling-explorations/sonobe), a library for folding-based IVC. We forked Sonobe and built a variant of Nova with support for lookup arguments in the [`folding-schemes`](https://github.com/winderica/eva/tree/video/folding-schemes) folder. The code for video authentication can be found in [`video`](https://github.com/winderica/eva/tree/video/video).

## Prerequisites

Hardware requirements:
- 64 GB of RAM.
- 50 GB of free disk space.

You need to have packages for C/C++ development (`build-essential` for `apt`, `development-tools` for `dnf`, `base-devel` for `pacman`) and [Rust](https://www.rust-lang.org/tools/install) installed on your machine.

The default build uses a CPU backend. For faster MSM and matrix-vector operations on Nvidia GPUs, build with `--features cuda` (requires [CUDA](https://docs.nvidia.com/cuda/cuda-installation-guide-linux/); we used an RTX 3080 with 12 GB of VRAM).

Before running the examples, download the `data_parsed` folder from Hugging Face and set the environment variable `DATA_PATH=/<path>/<to>/data_parsed`.

You can also use your own video, but you need to extract the original & prediction macroblocks as well as the quantized coefficients from the JM library. Instructions for dataset preparation will be provided in the future.

## How to run

Simply execute `cargo run --release --example=<example>`.
