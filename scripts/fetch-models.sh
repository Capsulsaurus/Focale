#!/bin/sh
# fetch-models.sh — download the AI segmentation models Focale uses.
#
# Focale itself makes NO network calls (PRD §2.3, local-only). This script is
# the one sanctioned download path: run it once, by hand, and the app loads
# the models from the user data directory forever after. Every URL is pinned
# and verified against a sha256 recorded below.
#
# Target directory: $XDG_DATA_HOME/focale/models (default
# ~/.local/share/focale/models), matching focale_segment::ModelPaths.
#
# Models and licenses (all AGPL-compatible, verified 2026-07-11):
#
#   mobile_sam_image_encoder.onnx / sam_mask_decoder_single.onnx
#     MobileSAM (https://github.com/ChaoningZhang/MobileSAM), Apache-2.0.
#     ONNX export from https://huggingface.co/Acly/MobileSAM (repo tagged
#     MIT). Encoder embeds SAM preprocessing (mean/std + padding); decoder is
#     the standard single-mask SAM decoder export.
#
#   face_parsing_resnet18.onnx
#     BiSeNet face parsing, ResNet-18 backbone, MIT.
#     https://github.com/yakhyo/face-parsing (weights retrained from
#     zllrunning/face-parsing.PyTorch, also MIT). 19 CelebAMask-HQ classes.
#
#   u2net.onnx
#     U²-Net salient object detection, Apache-2.0
#     (https://github.com/xuebinqin/U-2-Net). ONNX from the rembg model
#     release (https://github.com/danielgatis/rembg, MIT).
#
#   skyseg.onnx
#     U²-Net sky segmentation, MIT.
#     https://huggingface.co/JianyuanWang/skyseg, model from
#     https://github.com/xiongzhu666/Sky-Segmentation-and-Post-processing.

# pipefail is POSIX.1-2024; supported by dash >= 0.5.12, bash, and busybox sh.
set -eu -o pipefail

data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
models_dir="$data_home/focale/models"
mkdir -p "$models_dir"

# fetch <file name> <sha256> <url>
fetch() {
    name="$1"
    sha="$2"
    url="$3"
    dest="$models_dir/$name"
    if [ -f "$dest" ] && printf '%s  %s\n' "$sha" "$dest" | sha256sum -c - >/dev/null 2>&1; then
        echo "ok       $name (already present)"
        return 0
    fi
    echo "fetching $name"
    tmp="$dest.part"
    curl --fail --location --continue-at - --output "$tmp" "$url"
    printf '%s  %s\n' "$sha" "$tmp" | sha256sum -c - >/dev/null
    mv "$tmp" "$dest"
    echo "ok       $name"
}

# MobileSAM image encoder (Apache-2.0 model, MIT export repo)
fetch mobile_sam_image_encoder.onnx \
    580f5fb648ea1062c0aabc26217aed56921985f03f0cbbd852bba81d760cc749 \
    https://huggingface.co/Acly/MobileSAM/resolve/main/mobile_sam_image_encoder.onnx

# SAM mask decoder, single-mask variant (Apache-2.0 model, MIT export repo)
fetch sam_mask_decoder_single.onnx \
    93915fc7c993ab9d59ab8c9ccd3bce37f7509c81ab4150a74abd4d2abbd8570d \
    https://huggingface.co/Acly/MobileSAM/resolve/main/sam_mask_decoder_single.onnx

# BiSeNet face parsing, ResNet-18 (MIT)
fetch face_parsing_resnet18.onnx \
    0d9bd318e46987c3bdbfacae9e2c0f461cae1c6ac6ea6d43bbe541a91727e33f \
    https://github.com/yakhyo/face-parsing/releases/download/weights/resnet18.onnx

# U²-Net saliency (Apache-2.0)
fetch u2net.onnx \
    8d10d2f3bb75ae3b6d527c77944fc5e7dcd94b29809d47a739a7a728a912b491 \
    https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2net.onnx

# U²-Net sky segmentation (MIT)
fetch skyseg.onnx \
    ab9c34c64c3d821220a2886a4a06da4642ffa14d5b30e8d5339056a089aa1d39 \
    https://huggingface.co/JianyuanWang/skyseg/resolve/main/skyseg.onnx

echo "all models installed in $models_dir"
