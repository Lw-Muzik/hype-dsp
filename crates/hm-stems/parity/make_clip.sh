#!/usr/bin/env bash
# Synthesise a rhythmic test clip (kick, hats, chord pad, vibrato "voice") as
# interleaved stereo float32 @ 44.1 kHz.  make_clip.sh OUT.f32 [SECONDS]
set -euo pipefail
out="$1"; secs="${2:-40}"
ffmpeg -v error -y \
 -f lavfi -i "aevalsrc=0.7*sin(2*PI*55*mod(t\,0.5)*(1+8*exp(-20*mod(t\,0.5))))*exp(-6*mod(t\,0.5)):s=44100:d=$secs" \
 -f lavfi -i "aevalsrc=0.3*(random(0)*2-1)*exp(-40*mod(t\,0.25)):s=44100:d=$secs" \
 -f lavfi -i "aevalsrc=0.12*(sin(2*PI*110*t)+0.5*sin(2*PI*220*t)+sin(2*PI*277*t)+sin(2*PI*330*t)+0.3*sin(2*PI*1320*t)):s=44100:d=$secs" \
 -f lavfi -i "aevalsrc=0.2*sin(2*PI*(440+25*sin(2*PI*5*t))*t)*(0.6+0.4*sin(2*PI*0.7*t))+0.05*sin(2*PI*14000*t):s=44100:d=$secs" \
 -filter_complex "[0][1][2][3]amix=inputs=4:normalize=0,pan=stereo|c0=c0|c1=0.9*c0,aformat=sample_fmts=flt:sample_rates=44100" \
 -f f32le "$out"
