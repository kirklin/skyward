#!/usr/bin/env python3
"""Read stock images, extracting only odm/vendor; discard the temporary raw super."""

import argparse
import hashlib
import json
import shutil
import subprocess
import tempfile
from pathlib import Path


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        # Small reads also work on WSL's DrvFS with large EROFS images.
        for block in iter(lambda: stream.read(64 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--images", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--filesystem-out", type=Path,
                        help="filesystem extraction root; use native Linux storage under WSL")
    parser.add_argument("--reuse-partitions", action="store_true",
                        help="reuse both already extracted partition images without reading super again")
    parser.add_argument("--reuse-filesystems", action="store_true",
                        help="finish auditing an interrupted run whose two filesystems are already extracted")
    parser.add_argument("--simg2img", default="simg2img")
    parser.add_argument("--lpunpack", default="lpunpack")
    parser.add_argument("--fsck-erofs", default="fsck.erofs")
    args = parser.parse_args()
    if args.reuse_filesystems and not args.reuse_partitions:
        parser.error("--reuse-filesystems also requires --reuse-partitions")
    source, output = args.images.resolve(strict=True), args.out.resolve()
    filesystem_out = (args.filesystem_out or output).resolve()
    for destination in (output, filesystem_out):
        if destination == source or source in destination.parents or destination in source.parents:
            parser.error("output and official images must be separate directories")
    inputs = [source / name for name in ("super.img", "recovery.img")]
    before = {str(p): (p.stat().st_size, p.stat().st_mtime_ns) for p in inputs}
    output.mkdir(parents=True, exist_ok=True)
    filesystem_out.mkdir(parents=True, exist_ok=True)
    partitions = output / "partitions"
    partitions.mkdir(exist_ok=True)
    destinations = [partitions / f"{name}_a.img" for name in ("odm", "vendor")]
    if not args.reuse_partitions and any(p.exists() for p in destinations):
        parser.error("partition outputs already exist; use a new output directory")
    # The input is opened by simg2img for reading only. TemporaryDirectory owns
    # the sole raw super and removes it on success or a subprocess failure.
    if args.reuse_partitions:
        if not all(p.is_file() for p in destinations):
            parser.error("--reuse-partitions requires both odm_a.img and vendor_a.img")
    else:
        with tempfile.TemporaryDirectory(prefix="skyward-sheng-super-") as work:
            raw = Path(work) / "super.raw.img"
            print(f"Unsparse read-only source into temporary file: {raw}", flush=True)
            subprocess.run([args.simg2img, str(inputs[0]), str(raw)], check=True)
            subprocess.run([args.lpunpack, "-p", "odm_a", "-p", "vendor_a",
                            str(raw), str(partitions)], check=True)
    print("Temporary raw super removed; official super preserved.", flush=True)
    for name, image in zip(("odm", "vendor"), destinations):
        root = filesystem_out / name
        if args.reuse_filesystems:
            if not (root / "etc").is_dir():
                parser.error(f"existing filesystem is missing etc/: {root}")
            subprocess.run([args.fsck_erofs, str(image)], check=True)
            continue
        if root.exists():
            parser.error(f"extraction directory already exists: {root}")
        subprocess.run([args.fsck_erofs, f"--extract={root}", str(image)], check=True)
    recovery = output / "recovery.img"
    if recovery.exists() and not args.reuse_filesystems:
        parser.error(f"recovery output already exists: {recovery}")
    if args.reuse_filesystems:
        if not recovery.is_file() or sha256(recovery) != sha256(inputs[1]):
            parser.error("existing recovery differs from the read-only source")
    else:
        shutil.copyfile(inputs[1], recovery)
    after = {str(p): (p.stat().st_size, p.stat().st_mtime_ns) for p in inputs}
    if before != after:
        raise RuntimeError("source image metadata changed during extraction")
    report = {
        "source": str(source), "source_opened_read_only": True,
        "source_metadata_unchanged": before == after,
        "temporary_raw_super_removed": True,
        "super_partitions_extracted": ["odm_a", "vendor_a"],
        "filesystem_root": str(filesystem_out),
        "reused_filesystems": args.reuse_filesystems,
        "sha256": {str(p.relative_to(output)): sha256(p)
                   for p in [recovery, *destinations]},
    }
    (output / "provenance.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2), flush=True)


if __name__ == "__main__":
    main()
