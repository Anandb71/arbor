"""A Bedrock client from a boto3 session, passed to a helper (synthetic)."""
import boto3

session = boto3.Session(profile_name="research")
bedrock = session.client("bedrock-runtime")


def converse_with(client, model_id, text):
    return client.converse(
        modelId=model_id,
        messages=[{"role": "user", "content": [{"text": text}]}],
    )


converse_with(bedrock, "mistral.mistral-large-2402-v1:0", "hello")
