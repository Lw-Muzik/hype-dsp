import { create } from "zustand";

/** One pending yes/no question, answered by `<ConfirmHost />`. Lets non-React
 *  code (the play router) ask without owning any UI. */
interface ConfirmStore {
  question: { message: string; resolve: (yes: boolean) => void } | null;
  ask(message: string): Promise<boolean>;
  answer(yes: boolean): void;
}

export const useConfirmStore = create<ConfirmStore>((set, get) => ({
  question: null,
  ask: (message) =>
    new Promise<boolean>((resolve) => {
      // A newer question replaces an unanswered one (counted as "no").
      get().question?.resolve(false);
      set({ question: { message, resolve } });
    }),
  answer: (yes) => {
    const q = get().question;
    set({ question: null });
    q?.resolve(yes);
  },
}));

export const confirmAsk = (message: string): Promise<boolean> => useConfirmStore.getState().ask(message);
