# Prove a Managed Worktree by a marker rather than its location

Reclaim took any linked Worktree standing directly under the managed directory for a Managed Worktree. That was safe enough while the directory was `.suru-worktrees`, a name nobody else would choose. It is now `.worktrees`, which people already use for Worktrees they make by hand, and Reclaim removes a Worktree's ignored files with it, so location alone would let an unattended sweep destroy a hand-made Worktree's `.env` the moment no Session referenced it.

Suru now records that it made a Worktree in a `suru-managed` file in the Worktree's Git registration, `<common>/worktrees/<id>/`, and Reclaim requires that file as well as the location. Preparation writes it straight after `git worktree add`, ahead of anything that can fail, so a Worktree left by a failed preparation can still be Reclaimed. Recovery writes it again, because the removal that recovery undoes took the first one with the registration.

Location still decides how a Worktree is presented, because a mistake there costs only a shorter label.

## Considered Options

- **Reuse `suru-preparation`**: it already stands in every prepared Worktree's registration, but recovery cannot write it, since its value must match a preparation intent, so a recovered Worktree would never be Reclaimed again.
- **Read `branch.<name>.suru-base`**: this proves a branch rather than a Worktree. It is missing on a detached Worktree and misleading on one switched to another branch.
- **A marker in the checkout, or a `.gitignore` beside it**: rejected because it adds to what the reader sees and can be committed by accident. The registration is outside the working tree, and Git removes it with the Worktree.
- **A record in the Server's database**: rejected because a Repository's Worktrees must be judged after a restart from what the Repository itself holds, and this would add a second source of truth for what Git already tracks.

## Consequences

A hand-made Worktree in the managed directory is never Reclaimed and never needs a `git worktree lock`. One that a Session worked in and whose directory later vanished is recreated by recovery, carries the marker from then on, and may be Reclaimed: Suru made that copy, and its ignored files died with the original.

A source control system that cannot tell keeps the default answer of `false` and has nothing removed unattended.
