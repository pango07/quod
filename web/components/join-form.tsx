"use client";

import { useActionState, useId } from "react";
import { joinWaitlist, type JoinState } from "@/app/actions";

const initial: JoinState = { status: "idle", message: "" };

export function JoinForm({ source }: { source: string }) {
  const [state, action, pending] = useActionState(joinWaitlist, initial);
  const id = useId();
  const done = state.status === "joined" || state.status === "already";
  return (
    <div className="join-wrap">
      <form className="join" action={action}>
        <label htmlFor={id} className="sr-only">
          Work email
        </label>
        <input
          id={id}
          name="email"
          type="email"
          required
          autoComplete="email"
          placeholder="you@company.com"
          disabled={done}
        />
        <input type="hidden" name="source" value={source} />
        <button className="btn" type="submit" disabled={pending || done}>
          {done ? "Joined" : pending ? "Joining…" : "Join the waitlist"}
        </button>
      </form>
      <p className={`join-msg ${state.status === "error" ? "err" : ""}`} aria-live="polite">
        {state.message}
      </p>
    </div>
  );
}
