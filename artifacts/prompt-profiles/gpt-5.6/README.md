# GPT-5.6 stock prompts

These snapshots document the stock prompt for [GPT-5.6 Sol](gpt-5.6-sol--stock.md), [Terra](gpt-5.6-terra--stock.md), and [Luna](gpt-5.6-luna--stock.md). They include the captured catalog template and describe how the request incorporates runtime permissions, tools, workspace instructions, and conversation history.

The stock path uses the resolved model catalog entry for every model. GPT-6.1 Sol and Terra already have their own templates in the bundled catalog. Provider-supplied models use their catalog entries when available.

Existing `experimental_prompt_profile` values load as `stock`. Older custom option tables are accepted for compatibility and have no effect on prompt assembly.
