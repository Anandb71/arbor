"""Triage agents built with the OpenAI Agents SDK."""
from agents import Agent, Runner, function_tool, OpenAIChatCompletionsModel
from agents.extensions.models.litellm_model import LitellmModel
from openai import AsyncOpenAI

WRITER_MODEL = "gpt-4o-mini"

triage = Agent(name="triage", instructions="Route the request.", model="gpt-4o")
writer = Agent(name="writer", model=WRITER_MODEL)
fallback = Agent(name="fallback")

local = AsyncOpenAI(base_url="http://localhost:11434/v1", api_key="ollama-local-not-a-key")
llama = OpenAIChatCompletionsModel(model="llama3.1", openai_client=local)
offline = Agent(name="offline", model=llama)

claude = LitellmModel(model="anthropic/claude-3-5-haiku-20241022")
reviewer = Agent(name="reviewer", model="litellm/anthropic/claude-3-5-sonnet-20241022")


@function_tool
def lookup(order_id: str) -> str:
    return order_id


async def handle(question):
    return await Runner.run(triage, question)
