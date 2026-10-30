import OpenAI from "openai";

const client = new OpenAI();
const proxied = client.withOptions({ baseURL: "http://localhost:4000/v1" });

const PARAMS = { model: "gpt-4o-mini", temperature: 0 } as const;

const MODELS = {
  fast: "gpt-4o-mini",
  smart: "gpt-4o",
};

async function ask(prompt: string, model = "gpt-4-turbo") {
  return client.chat.completions.create({ model, messages: [{ role: "user", content: prompt }] });
}

export async function run(prompt: string) {
  await ask(prompt);
  await client.chat.completions.create({ ...PARAMS, messages: [] });
  await client.chat.completions.create({ model: MODELS.smart, messages: [] });
  await proxied.chat.completions.create({ model: "gpt-4o", messages: [] });
  return send(client, prompt);
}

export async function pick(tier: keyof typeof MODELS) {
  return client.chat.completions.create({ model: MODELS[tier], messages: [] });
}

export function send(llm: OpenAI, prompt: string) {
  return llm.chat.completions.create({ model: "gpt-4o", messages: [{ role: "user", content: prompt }] });
}

export function withDefault(options: { model?: string }) {
  return client.chat.completions.create({ model: options.model || "gpt-4o", messages: [] });
}
