"""Raw HTTP calls to OpenAI's REST API with requests (synthetic)."""
import json
import os

import requests

API_KEY = os.environ["OPENAI_API_KEY"]
CHAT_URL = "https://api.openai.com/v1/chat/completions"
HEADERS = {"Authorization": "Bearer " + API_KEY}


def chat(prompt):
    payload = {"model": "gpt-4o-mini", "messages": [{"role": "user", "content": prompt}]}
    response = requests.post(CHAT_URL, headers=HEADERS, json=payload, timeout=30)
    return response.json()["choices"][0]["message"]["content"]


def embed(text):
    body = {"model": "text-embedding-3-small", "input": text}
    return requests.post("https://api.openai.com/v1/embeddings", headers=HEADERS, data=json.dumps(body)).json()


def chat_with(model, prompt):
    return requests.post(CHAT_URL, headers=HEADERS, json={"model": model, "messages": []}).json()


def list_models():
    return requests.get("https://api.openai.com/v1/models", headers=HEADERS).json()
