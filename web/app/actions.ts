"use server";

import { ConvexHttpClient } from "convex/browser";
import { Resend } from "resend";
import { api } from "@/convex/_generated/api";

export type JoinState = { status: "idle" | "joined" | "already" | "error"; message: string };

const convex = new ConvexHttpClient(process.env.NEXT_PUBLIC_CONVEX_URL!);

/** Adds an email to the waitlist (Convex), then to the Resend segment. */
export async function joinWaitlist(_prev: JoinState, form: FormData): Promise<JoinState> {
  const email = String(form.get("email") ?? "").trim();
  const source = String(form.get("source") ?? "site");
  if (!email) return { status: "error", message: "Enter your email to join." };

  let result;
  try {
    result = await convex.mutation(api.waitlist.join, { email, source });
  } catch {
    return { status: "error", message: "We couldn't save that right now. Try again in a minute." };
  }
  if (!result.ok) return { status: "error", message: "That doesn't look like an email address." };
  if (result.already) return { status: "already", message: "You're already on the list. We'll be in touch." };

  // The waitlist is saved; Resend is best-effort so a mail outage never loses a signup.
  const key = process.env.RESEND_API_KEY;
  if (key) {
    const resend = new Resend(key);
    const segment = process.env.RESEND_SEGMENT_ID;
    await resend.contacts
      .create({ email: email.toLowerCase(), segments: segment ? [{ id: segment }] : undefined })
      .catch(() => undefined);
    const from = process.env.RESEND_FROM;
    if (from) {
      await resend.emails
        .send({
          from,
          to: email,
          subject: "You're on the Quod waitlist",
          text:
            "Thanks for joining the Quod waitlist.\n\n" +
            "Quod reuses verified Vitest results when their recorded inputs are unchanged. " +
            "Changed and ineligible test files still run.\n\n" +
            "We're letting teams in a few at a time and will email you when there's a spot for yours.\n\n" +
            "See how it works: https://www.quodci.com/#how\n",
        })
        .catch(() => undefined);
    }
  }
  return { status: "joined", message: "You're on the list. We'll reach out when there's a spot." };
}
