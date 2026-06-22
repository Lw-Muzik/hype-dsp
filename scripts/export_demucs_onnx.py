#!/usr/bin/env python3
"""Export Demucs v4 (htdemucs) to an ONNX model matching hm-stems' contract.

The app's separator (`crates/hm-stems`) feeds a normalized stereo mixture
segment `[1, 2, L]` and expects four source segments `[1, 4, 2, L]` back, in the
slot order **vocals, drums, bass, other**. htdemucs natively outputs
`[drums, bass, other, vocals]`, so we wrap it to reorder, and we export the bare
model (no mean/std normalization — hm-stems does that in Rust).

Run on a machine with PyTorch + Demucs:

    pip install "torch>=2.1" demucs onnx onnxscript
    python3 scripts/export_demucs_onnx.py --out <stems>/model/htdemucs.onnx

Writes `htdemucs.onnx` + a sidecar `htdemucs.onnx.json` describing the I/O.

Note: htdemucs contains STFT/iSTFT, so export needs **opset 17+**. If your
PyTorch build can't trace iSTFT to ONNX, update PyTorch (2.1+ handles it) — this
is the one step that depends on your local toolchain.
"""
import argparse
import json
from fractions import Fraction
from pathlib import Path

import torch
from demucs.pretrained import get_model

# hm-stems' fixed playback-slot order.
TARGET_ORDER = ["vocals", "drums", "bass", "other"]


class Reordered(torch.nn.Module):
    """Wrap htdemucs so its output sources come out in TARGET_ORDER."""

    def __init__(self, model: torch.nn.Module, perm: list[int]):
        super().__init__()
        self.model = model
        self.register_buffer("perm", torch.tensor(perm, dtype=torch.long))

    def forward(self, mix: torch.Tensor) -> torch.Tensor:  # [B,2,L] -> [B,4,2,L]
        out = self.model(mix)
        return out.index_select(1, self.perm)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", default="htdemucs", help="Demucs model name")
    ap.add_argument("--out", required=True, help="output .onnx path")
    ap.add_argument("--opset", type=int, default=17)
    args = ap.parse_args()

    bag = get_model(args.model)
    bag.eval()
    # htdemucs ships as a one-model "bag"; take the inner model.
    model = bag.models[0] if hasattr(bag, "models") and bag.models else bag
    model.eval()

    sources = list(model.sources)  # e.g. ['drums','bass','other','vocals']
    perm = [sources.index(name) for name in TARGET_ORDER]

    # Segment length L in samples (htdemucs trains on 7.8 s @ 44.1 kHz).
    segment = getattr(model, "segment", Fraction(78, 10))
    sr = int(getattr(model, "samplerate", 44100))
    length = int(round(float(segment) * sr))

    wrapped = Reordered(model, perm).eval()
    dummy = torch.zeros(1, 2, length, dtype=torch.float32)

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with torch.no_grad():
        torch.onnx.export(
            wrapped,
            dummy,
            str(out_path),
            input_names=["mix"],
            output_names=["stems"],
            opset_version=args.opset,
            dynamic_axes=None,  # static shapes — CoreML prefers them
        )

    meta = {"input": "mix", "output": "stems", "segment": length,
            "samplerate": sr, "order": TARGET_ORDER}
    Path(str(out_path) + ".json").write_text(json.dumps(meta, indent=2))
    print(f"Wrote {out_path} (segment={length} samples, order={TARGET_ORDER})")
    print(f"Wrote {out_path}.json")


if __name__ == "__main__":
    main()
