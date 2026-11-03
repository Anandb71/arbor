import OpenAI from "openai";

export function makeClient() {
  return new OpenAI({ timeout: 20_000 });
}
