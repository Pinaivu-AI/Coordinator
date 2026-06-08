import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Pinaivu Dashboard",
  description: "API key management, usage analytics, and model catalog",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>
        <div className="min-h-screen flex">
          {/* Sidebar */}
          <aside className="w-56 shrink-0 bg-gray-900 border-r border-gray-800 flex flex-col py-6 px-4 gap-1">
            <div className="mb-6 px-2">
              <span className="text-lg font-bold tracking-tight text-white">
                pin<span className="text-indigo-400">aivu</span>
              </span>
              <p className="text-xs text-gray-500 mt-0.5">Developer Console</p>
            </div>
            <NavLink href="/"       label="Overview" />
            <NavLink href="/keys"   label="API Keys" />
            <NavLink href="/usage"  label="Usage" />
            <NavLink href="/models" label="Models" />
            <div className="mt-auto pt-4 border-t border-gray-800">
              <NavLink href="/setup" label="Setup" />
            </div>
          </aside>

          {/* Main */}
          <main className="flex-1 overflow-y-auto p-8">{children}</main>
        </div>
      </body>
    </html>
  );
}

function NavLink({ href, label }: { href: string; label: string }) {
  return (
    <a
      href={href}
      className="px-3 py-2 rounded-lg text-sm text-gray-400 hover:text-white hover:bg-gray-800 transition-colors"
    >
      {label}
    </a>
  );
}
