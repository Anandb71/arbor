import os

from anthropic import AnthropicBedrock, AnthropicVertex
from openai import AsyncOpenAI, AzureOpenAI

azure = AzureOpenAI(
    api_key=os.environ["AZURE_OPENAI_API_KEY"],
    api_version="2024-02-01",
    azure_endpoint=os.environ["AZURE_OPENAI_ENDPOINT"],
)
bedrock = AnthropicBedrock(aws_region="us-east-1")
vertex = AnthropicVertex(region="us-east5", project_id="demo-project")


def azure_chat(prompt):
    return azure.chat.completions.create(model="gpt35-prod-deployment", messages=[{"role": "user", "content": prompt}])


def bedrock_chat(prompt):
    return bedrock.messages.create(
        model="anthropic.claude-3-sonnet-20240229-v1:0", max_tokens=100, messages=[{"role": "user", "content": prompt}]
    )


def vertex_chat(prompt):
    return vertex.messages.create(
        model="claude-3-5-sonnet-v2@20241022", max_tokens=100, messages=[{"role": "user", "content": prompt}]
    )


async def stream(prompt):
    async with AsyncOpenAI() as client:
        return await client.chat.completions.create(
            model="gpt-4-turbo", stream=True, messages=[{"role": "user", "content": prompt}]
        )
