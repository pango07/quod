import { Logo } from "@/components/ui";

export default function Loading() {
  return (
    <main className="focus-page" aria-busy="true" style={{ alignItems: "center", paddingTop: 120 }}>
      <Logo size={28} />
      <span className="sr-only">Loading</span>
    </main>
  );
}
