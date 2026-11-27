# Third-party notices

Bragi is BSD-3-Clause (LICENSE). Its Rust and Dart dependencies carry their own licences in their
crates and packages. This file covers third-party programs shipped beside Bragi.

## FFmpeg 9.0.2 (ffmpeg and ffprobe)

The macOS app (`Bragi.app/Contents/Helpers`), the Linux and macOS tarballs and the Windows zip and
installer may carry `ffmpeg` and `ffprobe` from FFmpeg (https://ffmpeg.org/). Bragi runs them as
separate programs to read video clips and play them inline; it does not link to them. The deb, rpm,
Arch and Nix packages use the distribution's FFmpeg instead.

FFmpeg is licensed under the GNU Lesser General Public License, version 2.1 or (at your option) any
later version. The bundled build enables no GPL or non-free parts (`--disable-gpl --disable-nonfree`).
The full licence text ships beside the binaries as `ffmpeg/LICENSE.md` (`FFMPEG-LICENSE.md` on Windows)
and is at https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html.

FFmpeg is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the
implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Lesser General
Public License for more details.

**Source.** The unmodified release tarball https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz,
sha256 `8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e` (signed by the FFmpeg
release key FCF9 86EA 15E6 E293 A564 4F10 B432 2F04 D676 58D8). The build script is
[`scripts/build-ffmpeg.sh`](scripts/build-ffmpeg.sh). For three years from each Bragi release,
Weird Brains LLC will also send the source on request to m3talsmith@gmail.com.

**Build configuration** (`scripts/build-ffmpeg.sh --print-config`; cross builds add the target's
`--arch`/`--target-os`/`--cross-prefix` through `FFMPEG_CROSS`, recorded in `CONFIG.txt`):

```
./configure --disable-gpl --disable-nonfree --disable-version3 --disable-autodetect --disable-everything --disable-doc --disable-debug --disable-ffplay --disable-network --disable-iconv --disable-avdevice --enable-ffmpeg --enable-ffprobe --enable-small --enable-pic --enable-swscale --enable-swresample --enable-avfilter --enable-protocol=file,pipe --enable-demuxer=mov,matroska,avi,ogg,mp3,wav,aac --enable-decoder=h264,hevc,vp8,vp9,mpeg4,mpeg2video,mjpeg,theora,prores,aac,mp3,mp3float,opus,vorbis,flac,ac3,eac3,alac,pcm_s16le,pcm_s24le,pcm_f32le --enable-parser=h264,hevc,vp8,vp9,mpeg4video,mpegvideo,mjpeg,aac,mpegaudio,opus,vorbis,flac,ac3 --enable-bsf=h264_mp4toannexb,hevc_mp4toannexb,vp9_superframe_split --enable-encoder=mjpeg,pcm_s16le,wrapped_avframe --enable-muxer=image2pipe,wav,null --enable-filter=scale,fps,select,metadata,format,aformat,aresample,null,anull,trim,atrim,setpts,asetpts
```

You may replace the bundled binaries with any other ffmpeg and ffprobe: put yours in their place, or
point `BRAGI_FFMPEG` at an ffmpeg binary with ffprobe beside it.
