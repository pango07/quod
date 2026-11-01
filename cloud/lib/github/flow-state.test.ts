import { sealData } from "iron-session";
import { describe, expect, it } from "vitest";
import { checkFlowState, planInstalled, sealFlowState } from "./flow-state";

// Review M1: an attacker sends a team admin their own install-return URL
// (?code=&installation_id=, no state of ours). The code must never be used.

const PASSWORD = "p".repeat(32);
const q = (s: string) => new URLSearchParams(s);

describe("planInstalled", () => {
  it("drops a code that arrived without our state and starts our own OAuth instead", () => {
    expect(planInstalled(q("code=ATTACKER&installation_id=999&setup_action=install"), null)).toEqual({ kind: "authorize", installationId: 999, droppedCode: true });
  });

  it("uses a code only with a matching state", async () => {
    const trusted = await checkFlowState(await sealFlowState({ state: "S".repeat(32), userId: "user_1" }, PASSWORD), "S".repeat(32), "user_1", PASSWORD);
    expect(trusted).not.toBeNull();
    expect(planInstalled(q("code=MINE&installation_id=77&setup_action=install"), trusted)).toEqual({ kind: "verify", code: "MINE", installationId: 77 });
  });

  it("takes the installation id sealed with the OAuth state over the query", async () => {
    const trusted = await checkFlowState(await sealFlowState({ state: "T".repeat(32), userId: "user_1", installationId: 77 }, PASSWORD), "T".repeat(32), "user_1", PASSWORD);
    expect(planInstalled(q("code=MINE&installation_id=999"), trusted)).toEqual({ kind: "verify", code: "MINE", installationId: 77 });
  });

  it("handles request, update without a code, and a missing installation", () => {
    expect(planInstalled(q("setup_action=request"), null)).toEqual({ kind: "requested" });
    expect(planInstalled(q("installation_id=77&setup_action=update"), null)).toEqual({ kind: "check-linked", installationId: 77 });
    expect(planInstalled(q("code=X"), null)).toEqual({ kind: "no-installation" });
    expect(planInstalled(q("installation_id=abc&code=X"), null)).toEqual({ kind: "no-installation" });
  });
});

describe("checkFlowState", () => {
  it("refuses another user's state, a different state, a tampered or foreign blob", async () => {
    const sealed = await sealFlowState({ state: "S".repeat(32), userId: "user_1" }, PASSWORD);
    expect(await checkFlowState(sealed, "S".repeat(32), "user_2", PASSWORD)).toBeNull();
    expect(await checkFlowState(sealed, "X".repeat(32), "user_1", PASSWORD)).toBeNull();
    expect(await checkFlowState(sealed, "S".repeat(31), "user_1", PASSWORD)).toBeNull();
    expect(await checkFlowState(sealed, null, "user_1", PASSWORD)).toBeNull();
    expect(await checkFlowState(undefined, "S".repeat(32), "user_1", PASSWORD)).toBeNull();
    expect(await checkFlowState(sealed.slice(0, -3) + "AAA", "S".repeat(32), "user_1", PASSWORD)).toBeNull();
    const foreign = await sealData({ purpose: "quod-gh-manifest", state: "S".repeat(32), userId: "user_1" }, { password: PASSWORD, ttl: 600 });
    expect(await checkFlowState(foreign, "S".repeat(32), "user_1", PASSWORD)).toBeNull();
  });
});
