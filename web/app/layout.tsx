import type { Metadata, Viewport } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";

const geistSans = Geist({ variable: "--font-geist-sans", subsets: ["latin"] });
const geistMono = Geist_Mono({ variable: "--font-geist-mono", subsets: ["latin"] });

export const metadata: Metadata = {
  title: "Quod: Faster Vitest CI. Fewer runner minutes.",
  description:
    "Explore how Quod reuses verified Vitest results, try the interactive demos, and follow the current benchmark evidence.",
  openGraph: {
    title: "Quod: Faster Vitest CI. Fewer runner minutes.",
    description: "Try Quod’s interactive Vitest replay demos and explore the benchmark history. Savings for the corrected engine are still being validated.",
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
