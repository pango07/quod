import { internalMutation, mutation } from "./_generated/server";
import { v } from "convex/values";

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** Adds an email to the waitlist. Joining twice is not an error. */
export const join = mutation({
  args: { email: v.string(), source: v.string() },
  handler: async (ctx, { email, source }) => {
    const normalized = email.trim().toLowerCase();
    if (normalized.length > 254 || !EMAIL.test(normalized)) {
      return { ok: false as const, reason: "invalid" as const };
    }
    const existing = await ctx.db
      .query("waitlist")
      .withIndex("by_email", (q) => q.eq("email", normalized))
      .unique();
    if (existing) return { ok: true as const, already: true };
    await ctx.db.insert("waitlist", {
      email: normalized,
      source: source.slice(0, 32),
      createdAt: Date.now(),
    });
    return { ok: true as const, already: false };
  },
});

/** Admin only (not callable from the site): removes an email, e.g. on request. */
export const remove = internalMutation({
  args: { email: v.string() },
  handler: async (ctx, { email }) => {
    const row = await ctx.db
      .query("waitlist")
      .withIndex("by_email", (q) => q.eq("email", email.trim().toLowerCase()))
      .unique();
    if (row) await ctx.db.delete(row._id);
    return Boolean(row);
  },
});
