"""LangChain's Bedrock chat models (synthetic)."""
from langchain_aws import BedrockEmbeddings, ChatBedrock, ChatBedrockConverse

chat = ChatBedrock(model_id="us.anthropic.claude-3-5-sonnet-20241022-v2:0", region_name="us-west-2")
converse = ChatBedrockConverse(model="amazon.nova-lite-v1:0", temperature=0)
embeddings = BedrockEmbeddings(model_id="amazon.titan-embed-text-v2:0")


def answer(question):
    return chat.invoke(question)


def draft(question):
    return converse.invoke(question)


def index(texts):
    return embeddings.embed_documents(texts)
