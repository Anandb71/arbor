"""Amazon Bedrock through boto3: a synthetic fixture for `arbor usages`."""
import os

import boto3

CLAUDE = "us.anthropic.claude-sonnet-4-5-20250929-v1:0"
REQUEST = {"modelId": "amazon.nova-pro-v1:0", "body": "{}"}

runtime = boto3.client("bedrock-runtime", region_name="us-east-1")
gateway = boto3.client("bedrock-runtime", endpoint_url="http://bedrock-proxy.internal.example.com:8080")
storage = boto3.client("s3")


def ask(question):
    return runtime.converse(
        modelId=CLAUDE,
        messages=[{"role": "user", "content": [{"text": question}]}],
        inferenceConfig={"maxTokens": 512},
    )


def summarize(text):
    return runtime.invoke_model(**REQUEST)


def through_gateway(prompt):
    return gateway.converse_stream(modelId="meta.llama3-70b-instruct-v1:0", messages=[])


def configured(prompt):
    return runtime.converse(modelId=os.environ["BEDROCK_MODEL_ID"], messages=[])


def archive(key, data):
    storage.put_object(Bucket="transcripts", Key=key, Body=data)
