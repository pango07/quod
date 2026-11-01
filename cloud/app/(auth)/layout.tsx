import "./auth.css";

// Our own sign-in pages (no hosted AuthKit UI). Public: the proxy lets them through.
export default function AuthLayout({ children }: { children: React.ReactNode }) {
  return children;
}
