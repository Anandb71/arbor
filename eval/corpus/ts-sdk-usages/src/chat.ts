import OpenAI from "openai";

const MODEL = "gpt-3.5-turbo";

const client = new OpenAI();
const router = new OpenAI({ baseURL: "https://openrouter.ai/api/v1" });

export async function answer(question: string) {
  const completion = await client.chat.completions.create({
    model: MODEL,
    messages: [{ role: "user", content: question }],
    max_tokens: 200,
  });
  return completion.choices[0].message.content;
}

export async function route(question: string) {
  return router.chat.completions.create({
    model: "openai/gpt-4o",
    messages: [{ role: "user", content: question }],
  });
}

export async function embed(text: string) {
  return client.embeddings.create({ model: "text-embedding-3-small", input: text });
}
