import type { Metadata, Viewport } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";

const geistSans = Geist({ variable: "--font-geist-sans", subsets: ["latin"] });
const geistMono = Geist_Mono({ variable: "--font-geist-mono", subsets: ["latin"] });

export const metadata: Metadata = {
  title: "Quod: Your CI already did this",
  description:
    "Quod replays any CI step whose inputs haven't changed, and shows you the proof. No config. No rewrite. Join the waitlist.",
  openGraph: {
    title: "Quod: Your CI already did this",
    description: "Quod replays any CI step whose inputs haven't changed, and shows you the proof.",
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
