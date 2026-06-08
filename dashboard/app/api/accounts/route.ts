import { NextRequest, NextResponse } from "next/server";
import { createAccount } from "~/lib/coordinator";

export async function POST(req: NextRequest) {
  try {
    const body = await req.json().catch(() => ({}));
    const account = await createAccount(body.email, body.wallet_addr);
    return NextResponse.json(account);
  } catch (e: unknown) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}
