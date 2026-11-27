#!/usr/bin/env bash
# Builds the LGPL ffmpeg and ffprobe that Bragi's desktop packages bundle (docs/design/video.md,
# "Packaging"), reproducibly: a pinned upstream release tarball checked against its sha256, the
# configure line below and nothing else. Decoding only, plus the MJPEG, PCM and WAV output Bragi asks
# for; no GPL or nonfree parts, no network protocols, no external libraries.
#
#   scripts/build-ffmpeg.sh [OUT_DIR]           build into OUT_DIR (default: dist/ffmpeg/<triple>)
#   scripts/build-ffmpeg.sh --print-config      print the configure flags (for the notices) and exit
#
# Environment:
#   FFMPEG_SRC_DIR   where the tarball is kept between runs (default: ~/.cache/bragi-ffmpeg)
#   JOBS             make -j (default: 6)
#   FFMPEG_CROSS     extra configure flags, split on spaces (no quoting), e.g. for the x86_64 half of a
#                    macOS universal build on Apple silicon: "--enable-cross-compile --arch=x86_64
#                    --target-os=darwin --cc=clang --extra-cflags=--target=x86_64-apple-macos11
#                    --extra-ldflags=--target=x86_64-apple-macos11" (build both halves, then lipo; the
#                    Release workflow does), or "--extra-ldflags=-static" for a self-contained MSYS2 build
#
# OUT_DIR gets bin/ffmpeg and bin/ffprobe (.exe on Windows), LICENSE.md (LGPL 2.1 text from the source),
# CONFIG.txt (version, checksum and configure line) and SOURCE.txt (where the matching source is). The
# dist scripts copy these beside `bragi` (Contents/Helpers on macOS); Bragi finds them there before PATH.
# Needs a C compiler, make, nasm (x86) and curl. Shipping the binaries means offering this source: the
# tarball URL and checksum in SOURCE.txt are that offer, and THIRD_PARTY_NOTICES.md says so.
set -euo pipefail

FFMPEG_VERSION=9.0.2
FFMPEG_URL="https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz"
# sha256 of the release tarball, checked against the FFmpeg release signing key
# (FCF9 86EA 15E6 E293 A564 4F10 B432 2F04 D676 58D8) when it was pinned on 2026-11-26.
FFMPEG_SHA256=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e

# What Bragi's sampler and inline player read (crates/bragi-core/src/video.rs, video_play.rs): the
# containers it accepts (mp4, m4v, mov, webm, mkv, avi, ogv) and their usual codecs.
DEMUXERS=mov,matroska,avi,ogg,mp3,wav,aac
DECODERS=h264,hevc,vp8,vp9,mpeg4,mpeg2video,mjpeg,theora,prores,aac,mp3,mp3float,opus,vorbis,flac,ac3,eac3,alac,pcm_s16le,pcm_s24le,pcm_f32le
PARSERS=h264,hevc,vp8,vp9,mpeg4video,mpegvideo,mjpeg,aac,mpegaudio,opus,vorbis,flac,ac3
BSFS=h264_mp4toannexb,hevc_mp4toannexb,vp9_superframe_split
ENCODERS=mjpeg,pcm_s16le,wrapped_avframe
MUXERS=image2pipe,wav,null
FILTERS=scale,fps,select,metadata,format,aformat,aresample,null,anull,trim,atrim,setpts,asetpts

CONFIGURE_FLAGS=(
  --disable-gpl --disable-nonfree --disable-version3
  --disable-autodetect --disable-everything
  --disable-doc --disable-debug --disable-ffplay --disable-network --disable-iconv
  --disable-avdevice
  --enable-ffmpeg --enable-ffprobe
  --enable-small --enable-pic --enable-swscale --enable-swresample --enable-avfilter
  --enable-protocol=file,pipe
  "--enable-demuxer=$DEMUXERS" "--enable-decoder=$DECODERS" "--enable-parser=$PARSERS" "--enable-bsf=$BSFS"
  "--enable-encoder=$ENCODERS" "--enable-muxer=$MUXERS" "--enable-filter=$FILTERS"
)

if [[ "${1:-}" == "--print-config" ]]; then
  printf '%s\n' "ffmpeg $FFMPEG_VERSION ($FFMPEG_URL, sha256 $FFMPEG_SHA256)" "./configure ${CONFIGURE_FLAGS[*]} ${FFMPEG_CROSS:-}"
  exit 0
fi

cd "$(dirname "$0")/.."
root=$(pwd)
triple=$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')
out=$(realpath -m "${1:-dist/ffmpeg/${triple:-unknown}}")
src_dir="${FFMPEG_SRC_DIR:-$HOME/.cache/bragi-ffmpeg}"
tarball="$src_dir/ffmpeg-$FFMPEG_VERSION.tar.xz"
mkdir -p "$src_dir"
if [[ ! -f "$tarball" ]]; then
  curl -fL --retry 3 -o "$tarball.part" "$FFMPEG_URL"
  mv "$tarball.part" "$tarball"
fi
echo "$FFMPEG_SHA256  $tarball" | sha256sum -c - >/dev/null || {
  echo "ffmpeg tarball checksum mismatch: $tarball (delete it and try again)" >&2
  exit 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -C "$work" -xJf "$tarball"
cd "$work/ffmpeg-$FFMPEG_VERSION"
# Reproducible: no build date or host path in the binaries. ffmpeg compiles its configure line into the
# binaries (`ffmpeg -version`), so nothing on it names this machine: a fixed --prefix with DESTDIR, and an
# in-tree build (`./configure`), whose sources are compiled by relative path (no debug info).
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1764115200}"
extra_cflags=()
[[ -n "${CFLAGS:-}" ]] && extra_cflags=(--extra-cflags="$CFLAGS")
# shellcheck disable=SC2086 # FFMPEG_CROSS is a list of flags on purpose.
./configure --prefix=/usr/local ${extra_cflags[@]+"${extra_cflags[@]}"} "${CONFIGURE_FLAGS[@]}" ${FFMPEG_CROSS:-}
make -j"${JOBS:-6}"
make install DESTDIR="$work/stage"

ext=""
case "$triple" in *windows*) ext=".exe" ;; esac
mkdir -p "$out/bin"
install -m755 "$work/stage/usr/local/bin/ffmpeg$ext" "$out/bin/ffmpeg$ext"
install -m755 "$work/stage/usr/local/bin/ffprobe$ext" "$out/bin/ffprobe$ext"
install -m644 COPYING.LGPLv2.1 "$out/LICENSE.md"
{
  echo "ffmpeg $FFMPEG_VERSION, LGPL 2.1 or later, built by Bragi's scripts/build-ffmpeg.sh"
  echo "source: $FFMPEG_URL"
  echo "sha256: $FFMPEG_SHA256"
  echo "configure: ./configure ${CONFIGURE_FLAGS[*]} ${FFMPEG_CROSS:-}"
} > "$out/CONFIG.txt"
{
  echo "The ffmpeg and ffprobe binaries beside Bragi are FFmpeg $FFMPEG_VERSION (https://ffmpeg.org/), under the"
  echo "GNU Lesser General Public License 2.1 or later (LICENSE.md). Their complete source is the release"
  echo "tarball $FFMPEG_URL (sha256 $FFMPEG_SHA256), built unmodified with the configure line in CONFIG.txt"
  echo "by https://github.com/WeirdBrains/bragi/blob/main/scripts/build-ffmpeg.sh. They run as separate programs."
} > "$out/SOURCE.txt"
"$out/bin/ffmpeg$ext" -hide_banner -version | head -1 || true
cd "$root"
echo "ffmpeg built into $out"
