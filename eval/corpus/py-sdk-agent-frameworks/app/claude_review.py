"""Code review runs on the Claude Agent SDK."""
import os

from claude_agent_sdk import AgentDefinition, ClaudeAgentOptions, query

review_options = ClaudeAgentOptions(
    model="claude-sonnet-4-5-20250929",
    fallback_model="claude-haiku-4-5",
    env={"ANTHROPIC_API_KEY": os.environ["ANTHROPIC_API_KEY"]},
    agents={"linter": AgentDefinition(description="Lints", prompt="Lint the diff.", model="haiku")},
)
bedrock_options = ClaudeAgentOptions(model="claude-opus-4-1", env={"CLAUDE_CODE_USE_BEDROCK": "1"})


async def review(diff):
    async for message in query(prompt=diff, options=review_options):
        print(message)
