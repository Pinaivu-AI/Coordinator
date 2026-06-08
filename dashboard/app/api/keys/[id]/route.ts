import { NextRequest, NextResponse } from "next/server";
import { revokeKey } from "~/lib/coordinator";

export async function DELETE(
  _req: NextRequest,
  { params }: { params: { id: string } },
) {
  try {
    const result = await revokeKey(params.id);
    return NextResponse.json(result);
  } catch (e: unknown) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}
