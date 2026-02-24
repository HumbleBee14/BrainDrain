import type { Metadata } from "next";
import { ClerkProvider } from "@clerk/nextjs";
import { Inter } from "next/font/google";
import { Toaster } from "sonner";
import { Providers } from "./providers";
import "./globals.css";

const inter = Inter({ subsets: ["latin"] });

const appName = process.env.NEXT_PUBLIC_APP_NAME || "Platform";

export const metadata: Metadata = {
  title: appName,
  description: "Fine-tune LLMs on your data — no technical knowledge required.",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <ClerkProvider>
      <html lang="en">
        <body className={inter.className}>
          <Providers>{children}</Providers>
          <Toaster
            theme="dark"
            position="bottom-right"
            toastOptions={{
              style: {
                background: "#18181b",
                border: "1px solid #27272a",
                color: "#fafafa",
              },
            }}
          />
        </body>
      </html>
    </ClerkProvider>
  );
}
