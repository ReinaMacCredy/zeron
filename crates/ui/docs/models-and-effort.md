The model picker chooses which agent (Claude, Codex, and others) runs a chat, which model it uses, and how hard it thinks.

## Open the picker

The model chip sits in the composer next to the microphone and send button. It shows the model name and its reasoning level, plus a small bolt when fast mode is on. A model on an extra instance reads as the instance name, then the model, such as "Work · Opus" for an instance you named Work. Click it, or press Cmd+/ (**Open model picker**). Press Escape or click the chip again to close it.

If no agent is set up, the chip reads **No agents available**. Enable an installed agent in [Settings > Providers](zeron://settings/providers), or install an agent CLI.

## Compact picker

The compact picker is the default. It shows the current effort level in large type with the model name under it.

- Drag the slider, or press Left and Right, to change the reasoning effort. Home and End pick the lowest and highest level. On a Mac trackpad, each level change gives a small haptic tick.
- Click the bolt (**Fast mode**) or press F to turn fast mode on or off.
- Click the model name, or press Down or Enter, to open the model list. Search covers every provider, and tabs jump to each provider's group, including **Starred**. Click a row's star, or press Cmd+Shift+F on the highlighted model, to add it to favorites.
- Press Tab and Shift+Tab to cycle providers.

## Full picker

The full picker has a row of tabs along the top: a star for your favorites, a tab for agent profiles, then one tab per agent. Each agent's tab is followed by a tab for every extra instance you set up under **Add provider**, and ACP agents you added get tabs at the end. An instance's tab lists that instance's models, and picking one runs the chat on that instance. Search with **Search models…**. Click a row's star to add it to favorites. Cmd+1 to Cmd+9 pick the first nine rows.

Below the list, the **Reasoning** card shows the current level. Press Down past the last model to reach it, then Left and Right to change the level. A bolt button on the card turns fast mode on or off for models that have it, and a reset button restores the default level and speed.

Switch layouts with the button in the picker's header (**Switch to full picker** or **Switch to compact picker**), or with **Compact model picker** in [Settings > General](zeron://settings/general). **Effort slider** on the same page chooses **Glass** or **Stepped** for the slider.

## Reasoning levels

Levels depend on the model. They range from **Minimal** or **Low** up to **High**, **X-High**, and **Max**, and some models add **Ultra**, **Ultracode**, or **Ultrathink**. A new pick starts on **High** when the model offers it.

> In a chat, press Cmd+- or Cmd+= (Ctrl+- or Ctrl+= on Windows and Linux) to lower or raise the level without opening the picker. On macOS, **Thinking level shortcut** in Settings > General switches the modifier to Control.

## Switching in a running chat

Your pick applies to the chat you are in. Clicking another agent's tab only browses; nothing changes until you pick a model.

- **Same agent, new model**: if the chat has history, Zeron asks **Switch model?** The next response is slower and uses more tokens, because the full history is re-read. Choose **Yes, switch to …** or **No, go back** (press 1 or 2). A divider in the transcript marks the change.
- **Different agent or instance**: Zeron asks **Switch provider?** The old session is closed, and on your next message the conversation so far is handed to the new agent as plain text, the most recent 48,000 characters of what you and the agent wrote. Tool results, open files, and the old agent's cache are not carried over.

New chats start with the model you picked last for each agent.
