import { GoogleGenAI } from "@google/genai";
import { GoogleGenerativeAI } from "@google/generative-ai";

const genAI = new GoogleGenerativeAI(process.env.GEMINI_API_KEY ?? "");
const model = genAI.getGenerativeModel({ model: "gemini-1.5-flash" });

export async function legacy(prompt: string) {
  const result = await model.generateContent(prompt);
  const chat = model.startChat({ history: [] });
  await chat.sendMessage("and then?");
  return result.response.text();
}

const ai = new GoogleGenAI({ apiKey: process.env.GOOGLE_API_KEY });
const vertex = new GoogleGenAI({ vertexai: true, project: "acme", location: "us-central1" });

export async function current(prompt: string) {
  const response = await ai.models.generateContent({ model: "gemini-2.0-flash", contents: prompt });
  await vertex.models.generateContent({ model: "gemini-2.5-pro", contents: prompt });
  return response.text;
}
