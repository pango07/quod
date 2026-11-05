import type { Metadata, Viewport } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";

const geistSans = Geist({ variable: "--font-geist-sans", subsets: ["latin"] });
const geistMono = Geist_Mono({ variable: "--font-geist-mono", subsets: ["latin"] });

export const metadata: Metadata = {
  title: "Quod: Faster Vitest CI. Fewer runner minutes.",
  description:
    "Reuse verified Vitest results when their inputs are unchanged. See measured savings of 45.2% in runner time and 41.7% in estimated runner charges on convex-helpers.",
  openGraph: {
    title: "Quod: Faster Vitest CI. Fewer runner minutes.",
    description: "Measured on convex-helpers: 45.2% less runner time and 41.7% lower estimated runner charges across 12 commits, including the cold start.",
    images: ["/quod-logo.png"],
  },
};

export const viewport: Viewport = { themeColor: "#000000", colorScheme: "dark" };

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html lang="en" className={`${geistSans.variable} ${geistMono.variable}`}>
      <body>{children}</body>
    </html>
  );
}
