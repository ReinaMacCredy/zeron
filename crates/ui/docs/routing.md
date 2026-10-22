Routing decides where a delegated child runs when the agent gives it a role and no model. Set it once in [Settings > Routing](zeron://settings/routing): cheap, fast models for routine work, a strong model for checking it, and what happens when an agent runs out of quota. The same routes run the steps of a [Mission Control](zeron://docs/mission-control) plan.

## Roles

An agent names a role when it [delegates a task](zeron://docs/delegation) or plans a mission step. There are five:

| Role | For |
| --- | --- |
| **worker** | Writing and changing code, tests included |
| **validator** | Reviewing a diff or a finished step, running tests and QA |
| **research** | Reading code, docs and the web, and reporting back |
| **design** | Shaping an approach or a UI before code |
| **general** | Anything else |

Older names still work: **implementation** means worker, and **review** and **test** mean validator.

## The role table

Each role has a row with **Harness**, **Model** and **Reasoning**:

- **Off (agent decides)**, the default, leaves the choice to the agent, as before routing existed. Without a model, the child runs on the chat's own agent and model.
- **Same as parent** runs the child on the delegating chat's agent, model and reasoning level.
- An agent picks that agent, with the model and reasoning level beside it. If the agent is not installed or is turned off, the child runs on the parent's model instead and its chip says why.

Beside each model, a multiplier such as **0.04x** compares its price with the model named in the column header, from [models.dev](https://models.dev) prices, three input tokens to each output token. The **Quota** column shows how much of each agent's plan limit is used and when it resets, from [Usage](zeron://docs/usage). An agent that reports no usage shows **no quota data**: Zeron learns that it is out of quota from its first limit error, and then treats it as out for an hour, or until the reset time it gave.

Routes apply only when a child is created. Changing the table never moves a chat that is already running.

## What wins over a route

An agent can still choose: a `profile`, a `target.harness` or a `target.model` in its `delegate_task` call, or a harness or model for a mission step, always wins over the route. The child's chip then reads **chosen by agent** and names the route it skipped. A reasoning level alone keeps the route and only changes the reasoning.

Missions started with **Same as lead** skip routing for their steps. Code review children keep their own reviewer from [Settings > Code review](zeron://settings/code-review).

## When out of quota

Before a routed child starts, Zeron checks the routed agent's quota from what Usage last fetched. It never fetches on its own, and usage data more than 15 minutes old counts as unknown, so the child starts as routed. When the agent is out of quota, **When the routed harness is out of quota** decides:

- **Native sub-agent** (the default): a Claude Code chat runs the task as its own native sub-agent on its own model until the reset; no Zeron child opens. Other chats, mission steps, and chats whose native sub-agents are **Off** get a Zeron child on the chat's own model instead. When the chat's own agent is the one out of quota, the fallback list applies.
- **Next in fallback list**: the first agent in **Fallback list** that has quota. Out-of-quota entries are struck through. When none has quota, the child waits.
- **Wait for reset**: the child is created and holds its task until 15 seconds after the reset, then starts.
- **Ask me**: the agent is told the route is out of quota and asks you which of the three to take.

## When a child hits its limit mid-run

A delegated child or a mission step that hits its usage limit partway through gets a card under its chip in the parent's transcript: **Switch to** the chat's own model (or the first fallback with quota), which keeps the child's transcript and hands it to the new agent behind a provider divider, or **Wait for reset**, which resumes it on the same agent when the limit resets. **Mid-run limit** sets what happens without you: after **10 s** or **30 s** the preselected choice runs by itself, and **Ask** waits for you. **Cancel** stops the countdown. The child's own limit pill also offers **Switch to**. While the card waits, the parent's agent is not told the child failed.

Top-level chats keep the [usage limit pill](zeron://docs/usage) and auto-resume.

## Native sub-agents

**Claude Code native sub-agents** sets how Claude Code's own sub-agent tool fits in:

- **Only when same model** (the default): agents are told to use their native sub-agent only when the routed model is their own model, and `delegate_task` with the role otherwise. A quota fallback to **Native sub-agent** relaxes this for that one task.
- **Always allowed**: native sub-agents are fine for any work; `delegate_task` is for another agent or model.
- **Off**: Claude Code chats that Zeron starts lose their native sub-agent tool from their next turn, so they delegate with `delegate_task`, and a quota fallback opens a Zeron child on the chat's own model.

What **Off** does not cover: Codex, OpenCode and other agents' own sub-agent features, chats with the **Delegation** tools turned off in [Settings > Tools](zeron://settings/tools) (they keep native sub-agents so they can still delegate), a turn that is already running, and agents started by other MCP servers. **Only when same model** is guidance only; nothing enforces it per call.

**What agents are told** shows the delegation guidance the table produces, as each chat receives it on its next turn.

## In the transcript

Each delegated child's chip shows its role and model and a badge:

- **routed**: the child runs where Settings > Routing sends its role.
- **chosen by agent**: the agent passed a model or profile, which won over the route.
- **fell back**: the routed agent was out of quota or unavailable, with the reason, such as **Codex quota, resets 18:40**.

Hover the badge for the details and **Open Routing**. A task that fell back to a native sub-agent shows as one row with the same badge, followed by the agent's own sub-agent chip. A child that routing never touched has no badge. Mission step threads carry the same badges.

## The plan card

When a mission's lead writes its plan, the plan shows in the lead's transcript as a card before anything starts. Each step lists its role, the agent and model it will run on, and its multiplier. Below them:

- the mix of children per agent and model;
- **Relative cost**, a bar comparing the plan with running every step on the lead's model;
- a line for each step that falls back for quota, and a warning when the fallbacks raise the cost.

The multipliers are a floor: retries and reviews add runs. **Edit routing** opens Settings > Routing. **Start mission** approves the plan, as **Approve and run** does in Mission Control; it reads **Start with fallback** when a step falls back.
