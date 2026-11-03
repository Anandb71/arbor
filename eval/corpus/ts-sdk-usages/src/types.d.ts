import OpenAI from "openai";

declare global {
  var llm: OpenAI;
}

declare function complete(client: OpenAI): ReturnType<OpenAI["chat"]["completions"]["create"]>;
