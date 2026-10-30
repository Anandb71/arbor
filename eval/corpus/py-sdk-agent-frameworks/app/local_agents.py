"""A repository's own agents package, which is not the OpenAI Agents SDK."""
from agents import build_agent

support = build_agent(model="gpt-3.5-turbo-16k")
