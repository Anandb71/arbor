"""Model wrappers for AgentScope agents."""
import os

from agentscope.credential import OpenAICredential
from agentscope.model import AnthropicChatModel, DashScopeChatModel, OpenAIChatModel, OpenAIResponseModel

chat = OpenAIChatModel(model_name="gpt-4.1-mini", api_key=os.environ["OPENAI_API_KEY"], stream=True)
reasoner = OpenAIResponseModel(credential=OpenAICredential(api_key=os.environ["OPENAI_API_KEY"]), model="o3-mini")
proxied = AnthropicChatModel(
    model_name="claude-3-5-sonnet-20241022",
    client_kwargs={"base_url": "https://llm-gateway.internal.example.com"},
)
qwen = DashScopeChatModel(model_name="qwen-max", api_key=os.environ["DASHSCOPE_API_KEY"])
