import Anthropic from "@anthropic-ai/sdk";

const anthropic = new Anthropic();
await anthropic.messages.create({ model: "claude-3-haiku-20240307", max_tokens: 100, messages: [] });
