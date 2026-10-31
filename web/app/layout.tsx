import type { Metadata, Viewport } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";

const geistSans = Geist({ variable: "--font-geist-sans", subsets: ["latin"] });
const geistMono = Geist_Mono({ variable: "--font-geist-mono", subsets: ["latin"] });

export const metadata: Metadata = {
  title: "Quod: Skip the CI work you already did",
  description:
    "If nothing a CI step depends on has changed, Quod reuses its last result instead of running it again. Join the waitlist.",
  openGraph: {
    title: "Quod: Skip the CI work you already did",
    description: "If nothing a CI step depends on has changed, Quod reuses its last result instead of running it again.",
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
