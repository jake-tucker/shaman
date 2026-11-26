#!/bin/sh
# Throwaway (#774): makes the scratch Channel the prototype serves, from a copy of this machine's own Sessions.
#
#   make-scratch.sh <scratch-dir> <session-id>...
#
# The copy is taken with VACUUM INTO, which reads the live database as any reader does and writes nothing to it. Of
# the copy, only the Sessions named are kept; each is pointed at an empty directory beneath the scratch directory and
# cut loose from its parent, and everything else the copy knew of the machine (Workspaces, memories, Errands, Provider
# resume states, Attachments) is forgotten, so the Server that opens it reaches nothing of the user's.
set -eu
scratch=$1; shift
source_db=${SHAMAN_DB:-"$HOME/Library/Application Support/shaman/shaman.db"}
mkdir -p "$scratch/state/rune-transcript" "$scratch/workspace"
rm -f "$scratch/full.db"
sqlite3 "$source_db" ".timeout 10000" "VACUUM INTO '$scratch/full.db';"
keep=$(printf "'%s'," "$@"); keep=${keep%,}
sqlite3 "$scratch/full.db" "
pragma foreign_keys = on;
delete from sessions where id not in ($keep);
update sessions set parent_session_id = null, begun_by = null, brokered = 0, prompt_cache = null, settled_at = null,
  workspace = json_object(
    'workspace', json_object(
      'id', 'directory:0000000000000000000000000000000000000000000000000000000000000000',
      'path', '$scratch/workspace', 'repository', null,
      'source_control', json_object('status', 'not_detected'), 'icon', null, 'description', null),
    'checkout', null,
    'execution_directory', json_object('path', '$scratch/workspace'),
    'approval_posture', null);
delete from workspaces; delete from memories; delete from errands; delete from provider_resume_states;
delete from provider_subagent_identities; delete from attachments; delete from session_attachments;
delete from sidekick_acts; delete from clone_staging; delete from landing_agent_selection; delete from model_catalog;
vacuum;
"
sqlite3 "$scratch/full.db" "pragma integrity_check;" "select count(*) || ' Sessions kept' from sessions;"
mv "$scratch/full.db" "$scratch/pristine.db"
cp "$scratch/pristine.db" "$scratch/state/rune-transcript/shaman.db"
echo "export RT_STATE=$scratch/state"
