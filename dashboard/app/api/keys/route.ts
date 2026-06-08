import { NextRequest, NextResponse } from "next/server";
import { createKey, listKeys } from "~/lib/coordinator";

export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const key = await createKey(
      body.account_id,
      body.name,
      body.rpm_limit,
      body.daily_limit,
    );
    return NextResponse.json(key);
  } catch (e: unknown) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

export async function GET(req: NextRequest) {
  const accountId = req.nextUrl.searchParams.get("account_id") ?? "";
  try {
    const keys = await listKeys(accountId);
    return NextResponse.json(keys);
  } catch (e: unknown) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}
