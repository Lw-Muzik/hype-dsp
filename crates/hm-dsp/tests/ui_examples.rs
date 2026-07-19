//! The LiveProg examples the UI ships must compile.
//!
//! `ScriptCard.tsx` offers these as one-click starting points. An example that
//! errors the moment it is clicked is worse than offering none — it reads as the
//! feature being broken rather than the example being wrong, and it is the first
//! thing anyone will press.
//!
//! The sources are duplicated from `src/features/enhancer/ScriptCard.tsx`
//! because the VM is Rust and the card is TypeScript, and there is no shared
//! home worth building for three string literals. **Keep them in sync**: this
//! catches an example that stops compiling, not one that silently drifts from
//! what the card actually shows.

/// Kept as `(label, source)` so a failure names the button rather than a line
/// number in a fixture.
const EXAMPLES: &[(&str, &str)] = &[
    ("Gain", "@sample\n  spl0 = spl0 * 0.7;\n  spl1 = spl1 * 0.7;\n"),
    (
        "Tremolo",
        "@init\n  t = 0;\n@sample\n  t = t + 1;\n  \
         g = 0.5 + 0.5 * sin(t * 2 * $pi * 4 / srate);\n  \
         spl0 = spl0 * g;\n  spl1 = spl1 * g;\n",
    ),
    (
        "Soft clip",
        "@sample\n  spl0 = tanh(spl0 * 2) / tanh(2);\n  \
         spl1 = tanh(spl1 * 2) / tanh(2);\n",
    ),
];

#[test]
fn every_shipped_example_compiles() {
    for (label, source) in EXAMPLES {
        if let Err(e) = hm_dsp::script::compile(source) {
            panic!("the \"{label}\" example does not compile: {e}");
        }
    }
}

/// The examples are meant to *do* something. One that compiled to no `@sample`
/// work would still pass the test above while being a silent no-op in the card.
#[test]
fn every_shipped_example_actually_processes_audio() {
    for (label, source) in EXAMPLES {
        let program = hm_dsp::script::compile(source)
            .unwrap_or_else(|e| panic!("\"{label}\" does not compile: {e}"));
        assert!(
            !program.sample.is_empty(),
            "the \"{label}\" example emits no @sample ops — it would do nothing"
        );
    }
}
