# Migrate ngit-relay to ngit-grasp on NixOS VPS

**Goal:** Replace an ngit-relay instance on a VPS running NixOS with ngit-grasp.

**Specifics:** VPS running NixOS.

## Approach

1. Deploy ngit-grasp with 'domain' of `<prod-domain>.internal` and an `archiveService` of `<prod-domain>` running on a different port. This will gather all the events and git data from the production service and relays/git servers/grasp servers that for repositories that list the service in their announcement event. To sync all git data may take an hour.

2. Analyze the data to see which repositories have not been moved with complete data. Understand why and for each decide if action is needed / not needed to move it.

3. Set the 'domain' to production URL, turn off archive mode, and point your reverse proxy at the new port.

## Challenges

- **ngit-relay accepts any commits/annotated tags** that were at that point of time referenced in the latest state event. **ngit-grasp requires all the git data** to reproduce the latest state. So if the git data is incomplete, it won't accept the repository.

- **ngit-relay doesn't clear out refs/nostr/<event-id>** where it doesn't have a PR event. Fortunately the 'PR' (as opposed to patches) functionality is not widely used so we just need to check a few repositories (shakespeare, ngit and gitworkshop).

## Analysis Categories

### No action required:

- **Git Data Complete - Moved** (state event exists in archive and git data reflects it)
- **Invalid Repositories Announcement** (Won't Parse)
- **Deletion Request** (kind 5) tagging announcement event in archive
- **Announcement Not on Production But In Archive** that lists service

### Action/decision required:

- **Invalid State Event** (Won't Parse)
- **Incomplete Git Data** (at source and destination) And No State Event at Destination
- **No Announcement In Archive** (and no related delete event)
- **Complete Git Data at source, Announcement but no State Event in Archive** and empty bare git repo
- **State event but incomplete git data in Archive**

## Analysis Approach

This analysis and categorization should be scripted to facilitate easy review and decision making.

There are already some scripts that we need to build on in the old issue worktree to help facilitate this.

## Gotchas

Always use `nak req` with `--paginate` flag so we don't miss any events. If we receive increments of 250 eg 500 then it's a red flag that we are not paginating and there are probably more events.
