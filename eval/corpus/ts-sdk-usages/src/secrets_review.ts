import OpenAI from "openai";

const OPENAI_API_KEY = "sk-test-NOT-A-REAL-KEY-0123456789abcdef";
const client = new OpenAI({ apiKey: OPENAI_API_KEY });

const SECRET_MODEL = "gpt-4o";

export function leaks(authToken: string) {
  client.chat.completions.create({ model: SECRET_MODEL, messages: [] });
  client.chat.completions.create({ model: "sk-proj-EVALSECRETEVALSECRET0123", messages: [] });
  client.chat.completions.create({ model: authToken, messages: [] });
}
