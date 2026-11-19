"""Chat models served through the platform gateway and Azure OpenAI."""

from langchain_openai import AzureChatOpenAI, ChatOpenAI

GATEWAY_URL = "https://llm-gateway.internal.example.com/v1"

routed = ChatOpenAI(model="gpt-4o", base_url=GATEWAY_URL)
fallback = ChatOpenAI(temperature=0)
azure = AzureChatOpenAI(
    azure_deployment="support-gpt4o",
    model="gpt-4o",
    api_version="2024-06-01",
)


def route(prompt: str) -> str:
    return routed.invoke(prompt).content


def answer(question: str) -> str:
    try:
        return azure.invoke(question).content
    except TimeoutError:
        return fallback.invoke(question).content
