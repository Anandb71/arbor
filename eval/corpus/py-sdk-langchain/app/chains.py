"""Drafting, summary and review chains built on LangChain's partner packages."""

from langchain_anthropic import ChatAnthropic
from langchain_openai import ChatOpenAI, OpenAIEmbeddings

SUMMARY_MODEL = "gpt-4o-mini"

drafter = ChatOpenAI(model="gpt-4o", temperature=0.2, api_key="sk-test-NOT-A-REAL-KEY-langchain")
summarizer = ChatOpenAI(model_name=SUMMARY_MODEL, max_tokens=512)
reviewer = ChatAnthropic(model="claude-3-5-sonnet-20241022", max_tokens=1024)
embeddings = OpenAIEmbeddings(model="text-embedding-ada-002")


def draft(brief: str) -> str:
    return drafter.invoke(f"Write a first draft for: {brief}").content


def summarize(text: str) -> str:
    return summarizer.invoke(text).content


def review(draft_text: str) -> str:
    return reviewer.invoke(f"Review this draft:\n{draft_text}").content


def index(text: str) -> list[float]:
    return embeddings.embed_query(text)
