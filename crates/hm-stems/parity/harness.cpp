// Reference separation for the Rust parity test, built from the MOBILE engine's
// own sources: StereoStft (stft.cpp / fft.h), the Spleeter ONNX wrapper
// (spleeter_onnx.cpp) and stem_config.h. The per-chunk mask / overlap-add code
// below is `StemEngine::computeSplit` from stem_engine.cpp, copied verbatim
// except that it works on plain buffers instead of engine members.
//
//   harness MODEL_DIR input.f32 OUT_DIR CHUNK...
//
// input.f32 = interleaved stereo float32 at 44.1 kHz. For each grid chunk c
// writes OUT_DIR/chunk<c>.vocals.f32 and .drums.f32 (the owned range,
// interleaved stereo float32).
#include "separator.h"
#include "stem_config.h"
#include "stft.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <string>
#include <vector>
#include <unistd.h>

using namespace hype::stems;

static std::vector<float> readF32(const std::string& path) {
    std::ifstream f(path, std::ios::binary | std::ios::ate);
    if (!f) { std::fprintf(stderr, "cannot read %s\n", path.c_str()); std::exit(2); }
    size_t n = size_t(f.tellg()) / sizeof(float);
    f.seekg(0);
    std::vector<float> v(n);
    f.read(reinterpret_cast<char*>(v.data()), n * sizeof(float));
    return v;
}

static void writeF32(const std::string& path, const std::vector<float>& v) {
    std::ofstream f(path, std::ios::binary);
    f.write(reinterpret_cast<const char*>(v.data()), v.size() * sizeof(float));
}

int main(int argc, char** argv) {
    if (argc < 5) { std::fprintf(stderr, "usage: harness MODEL_DIR input.f32 OUT_DIR CHUNK...\n"); return 2; }
    const std::string modelDir = argv[1], inPath = argv[2], outDir = argv[3];
    const std::vector<float> mix = readF32(inPath);
    const int64_t frames = int64_t(mix.size() / 2);

    auto sep = createSpleeterOnnx(4);
    std::string err;
    if (!sep->load(modelDir, &err)) { std::fprintf(stderr, "load: %s\n", err.c_str()); return 1; }

    const size_t kBins = size_t(2) * kSplitFrames * kNetBins;
    StereoStft stft;
    std::vector<float> inL(kInputSpan), inR(kInputSpan), mag(kBins), eV(kBins), eD(kBins), tmp(kBins), sum(kBins);
    std::vector<float> avgV(2 * kSplitFrames), avgD(2 * kSplitFrames);
    std::vector<float> spec(size_t(4) * kSpecBins + size_t(8) * kSpecBins + size_t(4) * kNfft);
    std::vector<float> accVL(kInputSpan), accVR(kInputSpan), accDL(kInputSpan), accDR(kInputSpan);

    for (int a = 4; a < argc; ++a) {
        const int64_t c = std::atoll(argv[a]);
        const int64_t ownLo = splitOwnedStart(c);
        const int64_t ownHi = std::min<int64_t>(splitOwnedEnd(c), frames);
        const int64_t spanStart = chunkSpanStart(ownLo);
        for (int64_t i = 0; i < kInputSpan; ++i) {
            const int64_t at = spanStart + i;
            const bool in = at >= 0 && at < frames;
            inL[size_t(i)] = in ? mix[size_t(at) * 2] : 0.0f;
            inR[size_t(i)] = in ? mix[size_t(at) * 2 + 1] : 0.0f;
        }

        // ---- computeSplit (stem_engine.cpp) ----
        float* lRe = spec.data();
        float* lIm = lRe + kSpecBins;
        float* rRe = lIm + kSpecBins;
        float* rIm = rRe + kSpecBins;
        for (int j = 0; j < kSplitFrames; ++j) {
            stft.forward(&inL[size_t(j) * kHop], &inR[size_t(j) * kHop], lRe, lIm, rRe, rIm);
            float* mL = &mag[size_t(j) * kNetBins];
            float* mR = &mag[size_t(kSplitFrames + j) * kNetBins];
            for (int k = 0; k < kNetBins; ++k) {
                mL[k] = std::sqrt(lRe[k] * lRe[k] + lIm[k] * lIm[k]);
                mR[k] = std::sqrt(rRe[k] * rRe[k] + rIm[k] * rIm[k]);
            }
        }
        auto runNet = [&](int net, float* out) {
            if (!sep->run(net, mag.data(), out, &err)) { std::fprintf(stderr, "run: %s\n", err.c_str()); std::exit(1); }
        };
        const int kB = int(kBins);
        runNet(kNetVocals, eV.data());
        runNet(kNetDrums, eD.data());
        runNet(kNetBass, tmp.data());
        for (int i = 0; i < kB; ++i) sum[i] = eV[i] * eV[i] + eD[i] * eD[i] + tmp[i] * tmp[i];
        runNet(kNetOther, tmp.data());
        for (int i = 0; i < kB; ++i) {
            const float total = sum[i] + tmp[i] * tmp[i] + 1e-10f;
            eV[i] = (eV[i] * eV[i] + 2.5e-11f) / total;
            eD[i] = (eD[i] * eD[i] + 2.5e-11f) / total;
        }
        for (int row = 0; row < 2 * kSplitFrames; ++row) {
            double sv = 0.0, sd = 0.0;
            const float* mv = &eV[size_t(row) * kNetBins];
            const float* md = &eD[size_t(row) * kNetBins];
            for (int k = 0; k < kNetBins; ++k) { sv += mv[k]; sd += md[k]; }
            avgV[size_t(row)] = float(sv / kNetBins);
            avgD[size_t(row)] = float(sd / kNetBins);
        }
        std::fill(accVL.begin(), accVL.end(), 0.0f);
        std::fill(accVR.begin(), accVR.end(), 0.0f);
        std::fill(accDL.begin(), accDL.end(), 0.0f);
        std::fill(accDR.begin(), accDR.end(), 0.0f);
        float* vlRe = rIm + kSpecBins;
        float* vlIm = vlRe + kSpecBins;
        float* vrRe = vlIm + kSpecBins;
        float* vrIm = vrRe + kSpecBins;
        float* dlRe = vrIm + kSpecBins;
        float* dlIm = dlRe + kSpecBins;
        float* drRe = dlIm + kSpecBins;
        float* drIm = drRe + kSpecBins;
        float* fVL = drIm + kSpecBins;
        float* fVR = fVL + kNfft;
        float* fDL = fVR + kNfft;
        float* fDR = fDL + kNfft;
        for (int j = 0; j < kSplitFrames; ++j) {
            stft.forward(&inL[size_t(j) * kHop], &inR[size_t(j) * kHop], lRe, lIm, rRe, rIm);
            const float* mvL = &eV[size_t(j) * kNetBins];
            const float* mvR = &eV[size_t(kSplitFrames + j) * kNetBins];
            const float* mdL = &eD[size_t(j) * kNetBins];
            const float* mdR = &eD[size_t(kSplitFrames + j) * kNetBins];
            const float avL = avgV[size_t(j)], avR = avgV[size_t(kSplitFrames + j)];
            const float adL = avgD[size_t(j)], adR = avgD[size_t(kSplitFrames + j)];
            for (int k = 0; k < kSpecBins; ++k) {
                const bool net = k < kNetBins;
                const float gvl = net ? mvL[k] : avL, gvr = net ? mvR[k] : avR;
                const float gdl = net ? mdL[k] : adL, gdr = net ? mdR[k] : adR;
                vlRe[k] = lRe[k] * gvl; vlIm[k] = lIm[k] * gvl;
                vrRe[k] = rRe[k] * gvr; vrIm[k] = rIm[k] * gvr;
                dlRe[k] = lRe[k] * gdl; dlIm[k] = lIm[k] * gdl;
                drRe[k] = rRe[k] * gdr; drIm[k] = rIm[k] * gdr;
            }
            stft.inverse(vlRe, vlIm, vrRe, vrIm, fVL, fVR);
            stft.inverse(dlRe, dlIm, drRe, drIm, fDL, fDR);
            float* aVL = &accVL[size_t(j) * kHop];
            float* aVR = &accVR[size_t(j) * kHop];
            float* aDL = &accDL[size_t(j) * kHop];
            float* aDR = &accDR[size_t(j) * kHop];
            for (int t = 0; t < kNfft; ++t) { aVL[t] += fVL[t]; aVR[t] += fVR[t]; aDL[t] += fDL[t]; aDR[t] += fDR[t]; }
        }
        const float norm = 1.0f / StereoStft::kOlaGain;
        // ---- end computeSplit ----

        const int64_t n = ownHi - ownLo, first = ownLo - spanStart;
        std::vector<float> outV(size_t(n) * 2), outD(size_t(n) * 2);
        for (int64_t t = 0; t < n; ++t) {
            outV[size_t(t) * 2] = accVL[size_t(first + t)] * norm;
            outV[size_t(t) * 2 + 1] = accVR[size_t(first + t)] * norm;
            outD[size_t(t) * 2] = accDL[size_t(first + t)] * norm;
            outD[size_t(t) * 2 + 1] = accDR[size_t(first + t)] * norm;
        }
        writeF32(outDir + "/chunk" + std::to_string(c) + ".vocals.f32", outV);
        writeF32(outDir + "/chunk" + std::to_string(c) + ".drums.f32", outD);
        std::fprintf(stderr, "chunk %lld: %lld frames\n", (long long)c, (long long)n);
    }
    // ORT aborts in its static destructors at exit; skip them.
    std::fflush(nullptr);
    _exit(0);
}
