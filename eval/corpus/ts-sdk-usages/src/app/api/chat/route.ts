import { openai } from "@/lib/openai";

export async function POST(request: Request) {
  const { messages } = await request.json();
  const response = await openai.chat.completions.create({
    model: "gpt-4o-mini",
    stream: true,
    messages,
  });
  return new Response(response.toReadableStream());
}
