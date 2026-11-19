"""Billing support bot written against the pre-0.1 LangChain API."""

from langchain.chat_models import ChatOpenAI
from langchain.schema import HumanMessage, SystemMessage

chat = ChatOpenAI(model_name="gpt-3.5-turbo", temperature=0)


def reply(question):
    messages = [
        SystemMessage(content="You answer billing questions."),
        HumanMessage(content=question),
    ]
    return chat.predict_messages(messages).content
