#!/bin/sh
# Download the pydicom test corpus files the demo was verified against into
# samples/dicom/. They are synthetic, contain no real patient data, and are not
# committed to this repository.
set -eu
cd "$(dirname "$0")"
mkdir -p dicom
MAIN=https://raw.githubusercontent.com/pydicom/pydicom/main/src/pydicom/data/test_files
DATA=https://raw.githubusercontent.com/pydicom/pydicom-data/master/data_store/data
get() { url=$1; name=$(basename "$url"); [ -f "dicom/$name" ] || curl -fsSL -o "dicom/$name" "$url"; echo "$name"; }
for f in MR_small.dcm MR_small_RLE.dcm MR_small_implicit.dcm MR_small_bigendian.dcm MR_small_expb.dcm MR_small_padded.dcm \
         CT_small.dcm image_dfl.dcm MR_truncated.dcm no_meta.dcm \
         MR_small_jp2klossless.dcm MR_small_jpeg_ls_lossless.dcm JPEG2000.dcm 693_J2KI.dcm examples_jpeg2k.dcm \
         JPEG-lossy.dcm JPGExtended.dcm JPEGLSNearLossless_16.dcm \
         SC_rgb_rle.dcm SC_rgb_jpeg_dcmtk.dcm ExplVR_BigEnd.dcm examples_palette.dcm \
         rtdose.dcm rtdose_rle.dcm liver_1frame.dcm; do get "$MAIN/$f"; done
for f in JPEG-LL.dcm emri_small.dcm emri_small_RLE.dcm MR2_J2KR.dcm; do get "$DATA/$f"; done
