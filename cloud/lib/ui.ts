import "server-only";
import { cookies } from "next/headers";
import { pickUi, UI_COOKIE } from "./ui-version";

/** For a page: true when it renders the plain wording (lib/ui-version.ts says how it is chosen). */
export async function plainUi(searchParams?: Record<string, string | string[] | undefined>): Promise<boolean> {
  let cookie: string | undefined;
  try {
    cookie = (await cookies()).get(UI_COOKIE)?.value;
  } catch {
    cookie = undefined;
  }
  return pickUi(searchParams?.ui, cookie) === "plain";
}
