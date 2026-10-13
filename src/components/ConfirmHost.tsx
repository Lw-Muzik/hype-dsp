import { Button } from "@/components/Button";
import { Dialog } from "@/components/Dialog";
import { useConfirmStore } from "@/stores/confirm";

/** Renders the pending question from {@link useConfirmStore}, if any. */
export function ConfirmHost() {
  const question = useConfirmStore((s) => s.question);
  const answer = useConfirmStore((s) => s.answer);
  return (
    <Dialog open={question !== null} onClose={() => answer(false)} title="Only on this computer">
      <p className="text-sm text-text-muted">{question?.message}</p>
      <div className="mt-4 flex justify-end gap-2">
        <Button variant="ghost" onClick={() => answer(false)}>
          Not now
        </Button>
        <Button variant="primary" onClick={() => answer(true)} autoFocus>
          Use this computer
        </Button>
      </div>
    </Dialog>
  );
}
