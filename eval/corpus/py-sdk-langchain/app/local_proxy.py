"""Sends OpenAI chat models through a local LiteLLM proxy during development."""

import os

from langchain_anthropic import ChatAnthropic
from langchain_openai import ChatOpenAI

if os.getenv("USE_LOCAL_PROXY"):
    os.environ["OPENAI_API_BASE"] = "http://localhost:4000/v1"

assistant = ChatOpenAI(model="gpt-4o-mini")
critic = ChatAnthropic(model="claude-3-5-haiku-20241022")


def ask(prompt: str) -> str:
    return assistant.invoke(prompt).content


def critique(answer: str) -> str:
    return critic.invoke(answer).content
