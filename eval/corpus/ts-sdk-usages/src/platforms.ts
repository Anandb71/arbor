import { AzureOpenAI } from "openai";
import AnthropicBedrock from "@anthropic-ai/bedrock-sdk";
import { AnthropicVertex } from "@anthropic-ai/vertex-sdk";

const azure = new AzureOpenAI({
  endpoint: process.env.AZURE_OPENAI_ENDPOINT,
  apiVersion: "2024-10-21",
  deployment: "gpt-4o",
});

const bedrock = new AnthropicBedrock({ awsRegion: "us-west-2" });
const vertex = new AnthropicVertex({ region: "us-east5", projectId: "acme" });

export async function all(prompt: string) {
  await azure.chat.completions.create({ model: "gpt-4o", messages: [{ role: "user", content: prompt }] });
  await bedrock.messages.create({
    model: "anthropic.claude-3-5-sonnet-20241022-v2:0",
    max_tokens: 256,
    messages: [{ role: "user", content: prompt }],
  });
  await vertex.messages.create({
    model: "claude-3-5-sonnet-v2@20241022",
    max_tokens: 256,
    messages: [{ role: "user", content: prompt }],
  });
}
