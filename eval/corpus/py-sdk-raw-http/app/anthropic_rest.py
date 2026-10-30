"""Raw HTTP calls to Anthropic's REST API with httpx (synthetic)."""
import httpx
from httpx import post

MESSAGES_URL = "https://api.anthropic.com/v1/messages"


def ask(question, key):
    headers = {"x-api-key": key, "anthropic-version": "2023-06-01"}
    data = {"model": "claude-3-5-haiku-20241022", "max_tokens": 256, "messages": [{"role": "user", "content": question}]}
    return httpx.post(MESSAGES_URL, headers=headers, json=data).json()


def ask_default(question, key):
    return post(MESSAGES_URL, headers={"x-api-key": key}, json={"max_tokens": 256, "messages": []})


def ask_gateway(question):
    return httpx.post("https://llm-gateway.internal.example.com/v1/messages", json={"model": "claude-3-opus-20240229", "messages": []})
