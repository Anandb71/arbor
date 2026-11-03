import OpenAI from "openai";
import { DEFAULT_MODEL } from "./settings";
import { makeClient } from "./factory";

const client = new OpenAI();

export function fromEnv(prompt: string) {
  return client.chat.completions.create({
    model: process.env.OPENAI_MODEL ?? "gpt-4o",
    messages: [{ role: "user", content: prompt }],
  });
}

export function templated(version: string) {
  return client.chat.completions.create({ model: `gpt-${version}`, messages: [] });
}

export function imported() {
  return client.chat.completions.create({ model: DEFAULT_MODEL, messages: [] });
}

export function spread(options: Record<string, unknown>) {
  return client.chat.completions.create({ messages: [], ...options });
}

export function computed(method: "create" | "parse") {
  return client.chat.completions[method]({ model: "gpt-4o", messages: [] });
}

export function viaFactory() {
  const llm = makeClient();
  return llm.chat.completions.create({ model: "gpt-3.5-turbo", messages: [] });
}

export function viaParameter(api: any) {
  return api.responses.create({ model: "gpt-4.1", input: "hi" });
}

export const create = client.chat.completions.create;
