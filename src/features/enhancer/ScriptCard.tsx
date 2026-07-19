import { useState } from "react";
import { CircleAlert, Terminal } from "lucide-react";
import { Card } from "@/components/Card";
import { Switch } from "@/components/Switch";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import { ipcErrorMessage } from "@/lib/ipc";
import { toast } from "@/stores/toast";
import { cn } from "@/lib/cn";

/** Starting points, chosen to show one thing each: a bare `@sample`, state
 *  carried across frames in `@init`, and a builtin doing real work.
 *
 *  Every one of these compiles — checked against the VM, not assumed. An example
 *  that errors the moment it's clicked is worse than no example. */
const EXAMPLES = [
  {
    label: "Gain",
    code: "@sample\n  spl0 = spl0 * 0.7;\n  spl1 = spl1 * 0.7;\n",
  },
  {
    label: "Tremolo",
    code:
      "@init\n  t = 0;\n@sample\n  t = t + 1;\n" +
      "  g = 0.5 + 0.5 * sin(t * 2 * $pi * 4 / srate);\n" +
      "  spl0 = spl0 * g;\n  spl1 = spl1 * g;\n",
  },
  {
    label: "Soft clip",
    code:
      "@sample\n  spl0 = tanh(spl0 * 2) / tanh(2);\n" +
      "  spl1 = tanh(spl1 * 2) / tanh(2);\n",
  },
] as const;

/**
 * The LiveProg stage — a small EEL2-subset script run per audio frame.
 *
 * The editor is deliberately a plain textarea. Compile errors carry a line and
 * column and are shown as text; a line-number rail and inline markers were
 * considered and deferred until the feature has actually been used.
 *
 * Two separate things live here, and the distinction is the whole UI: the
 * **source** only reaches the chain when you press Apply, while the **toggle**
 * acts immediately on whatever was last applied. So editing text does not
 * silently change what you are hearing, and switching off does not lose your
 * work.
 */
export function ScriptCard() {
  const script = useEngineStore((s) => s.state.script);
  const setScriptEnabled = useEngineStore((s) => s.setScriptEnabled);
  const compileScript = useEngineStore((s) => s.compileScript);

  const [draft, setDraft] = useState(script.source);
  const [error, setError] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);

  const apply = async () => {
    setApplying(true);
    setError(null);
    try {
      await compileScript(draft);
      toast.success("Script applied");
    } catch (e) {
      // The chain keeps running whatever compiled last, so this is a report,
      // not a failure state to recover from.
      setError(ipcErrorMessage(e));
    } finally {
      setApplying(false);
    }
  };

  const dirty = draft !== script.source;

  return (
    <Card
      title="LiveProg (EEL2)"
      icon={Terminal}
      actions={
        <Switch
          checked={script.enabled}
          onChange={setScriptEnabled}
          label="Enable LiveProg script"
        />
      }
    >
      <div className={cn("flex flex-col gap-3", !script.enabled && "opacity-60")}>
        <textarea
          spellCheck={false}
          rows={8}
          value={draft}
          onChange={(e) => {
            setDraft(e.target.value);
            setError(null);
          }}
          aria-label="LiveProg script source"
          className={cn(
            "w-full resize-y rounded-control border bg-surface px-3 py-2 font-mono text-sm text-text",
            "placeholder:text-text-faint focus:outline-none",
            error ? "border-danger/60" : "border-border focus:border-accent/60",
          )}
          placeholder={"@init\n  // once, when the script loads\n@sample\n  // every frame — read and write spl0, spl1"}
        />

        {error && (
          <div className="flex items-start gap-2 rounded-control border border-danger/30 bg-danger/10 px-3 py-2 text-sm">
            <CircleAlert className="mt-0.5 size-4 shrink-0 text-danger" aria-hidden="true" />
            <span className="font-mono text-danger">{error}</span>
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <Button variant="primary" onClick={() => void apply()} disabled={applying || !dirty}>
            {applying ? "Applying…" : dirty ? "Apply" : "Applied"}
          </Button>

          <span className="ml-1 text-xs text-text-faint">Start from:</span>
          {EXAMPLES.map((ex) => (
            <button
              key={ex.label}
              type="button"
              onClick={() => {
                setDraft(ex.code);
                setError(null);
              }}
              className="rounded-control border border-border px-3 py-1.5 text-sm text-text-muted transition-colors hover:text-text"
            >
              {ex.label}
            </button>
          ))}
        </div>

        <p className="text-xs leading-relaxed text-text-faint">
          <span className="text-text-muted">spl0</span>,{" "}
          <span className="text-text-muted">spl1</span> are the left and right
          sample; <span className="text-text-muted">srate</span> is the sample
          rate. Constants <span className="text-text-muted">$pi</span>,{" "}
          <span className="text-text-muted">$e</span>. Functions: sin, cos, tan,
          asin, acos, atan, atan2, tanh, sqrt, exp, log, log10, abs, floor, ceil,
          round, sign, pow, min, max, fmod.
        </p>
      </div>
    </Card>
  );
}
