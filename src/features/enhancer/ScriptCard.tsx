import { useState } from "react";
import { Terminal, CircleAlert } from "lucide-react";
import { Card } from "@/components/Card";
import { Switch } from "@/components/Switch";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import { engineScriptCompile, ipcErrorMessage } from "@/lib/ipc";
import { toast } from "@/stores/toast";
import { cn } from "@/lib/cn";

// ─── example scripts ─────────────────────────────────────────────────────────
// Builtins confirmed from compiler.rs: sin, cos, tan, asin, acos, atan, sqrt,
// exp, log, log10, abs, floor, ceil, round, sign, tanh, atan2, pow, min, max, fmod

const EXAMPLES = [
  {
    label: "Gain",
    code: "spl0=spl0*0.7; spl1=spl1*0.7;",
  },
  {
    label: "Tremolo",
    code: "@init t=0;\n@sample t=t+1; g=0.5+0.5*sin(t*2*$pi*4/srate); spl0=spl0*g; spl1=spl1*g;",
  },
  {
    // tanh IS in the builtin table — Builtin::Tanh confirmed in compiler.rs line 85
    label: "Soft clip",
    code: "@sample spl0=tanh(spl0*2)/tanh(2); spl1=tanh(spl1*2)/tanh(2);",
  },
] as const;

// ─────────────────────────────────────────────────────────────────────────────

export function ScriptCard() {
  const script = useEngineStore((s) => s.state.script);
  const setScriptEnabled = useEngineStore((s) => s.setScriptEnabled);

  const [editorText, setEditorText] = useState(script.source);
  const [compileError, setCompileError] = useState<string | null>(null);
  const [compiling, setCompiling] = useState(false);

  const handleCompile = async () => {
    setCompiling(true);
    setCompileError(null);
    try {
      await engineScriptCompile(editorText);
      // Sync the compiled source back into the store so it persists.
      useEngineStore.setState((s) => ({
        state: { ...s.state, script: { ...s.state.script, source: editorText } },
      }));
      toast.success("Compiled");
    } catch (e) {
      setCompileError(ipcErrorMessage(e));
    } finally {
      setCompiling(false);
    }
  };

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
        {/* Editor */}
        <textarea
          spellCheck={false}
          rows={8}
          value={editorText}
          onChange={(e) => {
            setEditorText(e.target.value);
            setCompileError(null);
          }}
          className={cn(
            "w-full resize-y rounded-control border bg-bg-sunken px-3 py-2 font-mono text-sm text-text",
            "placeholder:text-text-faint focus:outline-none focus:ring-1 focus:ring-accent/50",
            compileError ? "border-danger/50" : "border-border",
          )}
          placeholder={"@init\n  // one-time setup\n@sample\n  // per-frame: edit spl0, spl1"}
        />

        {/* Compile error */}
        {compileError && (
          <div className="flex items-start gap-2 rounded-control border border-danger/30 bg-danger/10 px-3 py-2 text-sm">
            <CircleAlert
              className="mt-0.5 size-4 shrink-0 text-danger"
              aria-hidden="true"
            />
            <span className="font-mono text-danger">{compileError}</span>
          </div>
        )}

        {/* Actions row */}
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="primary"
            onClick={() => void handleCompile()}
            disabled={compiling}
          >
            {compiling ? "Compiling…" : "Compile"}
          </Button>

          <span className="text-xs text-text-faint">Examples:</span>

          {EXAMPLES.map((ex) => (
            <button
              key={ex.label}
              type="button"
              onClick={() => {
                setEditorText(ex.code);
                setCompileError(null);
              }}
              className="rounded-control border border-border px-3 py-1.5 text-sm text-text-muted transition-colors hover:text-text"
            >
              {ex.label}
            </button>
          ))}
        </div>

        <p className="text-xs text-text-faint">
          Builtins: sin, cos, tan, tanh, sqrt, abs, floor, ceil, round, exp, log,
          min, max, pow, atan2, fmod. Variables: spl0, spl1 (audio), srate
          (read-only).
        </p>
      </div>
    </Card>
  );
}
