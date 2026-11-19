# ngterm cookbook

Installable `ngtermctl` workflows. Each recipe is a directory holding a README and, where it needs one, its scripts: what it does, what it needs, how to set it up, and where it stops. They are written to be copied into your own setup and edited, not only read.

Every recipe is reviewed before it is accepted, but they are scripts you run on your own machine against your own sessions, so read a recipe before you run it.

ngterm also speaks agterm's control API, so most recipes in [agterm's cookbook](https://github.com/umputun/agterm/tree/master/cookbook) work here with `agtermctl` replaced by `ngtermctl`. This one holds the recipes written for ngterm.

## Recipes

Grouped by what you are trying to do, alphabetical within each group. Which agent a recipe needs is in its *needs* column.

### Workspaces and projects

| recipe | what it does | needs | author |
|---|---|---|---|
| [go-to-navigator](go-to-navigator/) | one chord opens a searchable tree of the window's workspaces, sessions and panes with each agent's status, and Enter jumps there | 0.1.0, jq, git | [@nickalie](https://github.com/nickalie) |

Most recipes need `ngtermctl`, which works in ngterm's own terminals and in commands a chord runs; outside them, put it on your `PATH` as the main README's *Get started* says. Each recipe's *Requirements* section names the minimum ngterm version it needs and any other tool it calls. Recipes are snapshots rather than a maintained surface: the control API grows by addition, so they rarely break.

## Contributing a recipe

[CONTRIBUTING.md](CONTRIBUTING.md) in this directory has the rules: ship a workflow you actually run, one kebab-case directory, the six-heading README template with its Author line, and an index row in the same pull request.

The bundled agent skill teaches the `ngtermctl` surface to a coding agent and carries its own worked examples in [`resources/skills/ngterm/examples.md`](../resources/skills/ngterm/examples.md); ngterm offers to install it on first launch.
