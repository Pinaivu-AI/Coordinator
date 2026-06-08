import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Coordinator API base URL — set COORDINATOR_URL in .env.local for dev,
  // or as an env var in production.
  env: {
    COORDINATOR_URL: process.env.COORDINATOR_URL ?? "https://13.206.80.190:4000",
  },
};

export default nextConfig;
