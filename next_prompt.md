Read DOCUMENTATION_INDEX.md and then the test strategy. We want to prove the concept of our architecture. Begin with writing the exportable test tool. Populate it with test related to the first line in (GRASP-01). "MUST serve a NIP-01 compliant nostr relay at / that accepts git repository announcements and their corresponding repo state announcements." Create the tests first and we will worry about the implemenation later. Can we cheat by reusing any rust-nostr tests for this? Suggest how much of NIP-01 we actually want to test based on the rust-nostr test, because this could potentially be quite a lot of work (thats not grasp specific, so we dont want to wate to much time on it, as most implemenations will use relay builders that have their own tests, maybe smoke tests are enough?).report back and ask me how to proceed.

Here was the prompt in response to the COMPLIANCE_TEST_PROPOSAL.md and you got started by creating the GRASP_AUDIT_PLAN.md and everything in grasp-audit: Option b: do build and test Nostr Relay features in paralell. use a seperate crate for tests instead of grasp-compliance-tests call it grasp-audit. We need to support isolated tests, running in parallel for cicd and tests that could be run to audit a production service, we could use specific tags or string in events to indicate they are audits can be cleaned up by a script regularly. another idea is to send deletion events but that leaves a trails of deletion events for the relay to store so the our other idea is better. Integrate that into the plan then try it out for the smoke tests and report back.

Next we will implement the OOTB relay to make these tests pass.

Then add line 2 test

"MUST reject git repository announcements that do not list the service in both clone and relays tags unless implementing GRASP-05"

next make these pass.

then prove out the git side of things....

we will do it step by step like this to begin with to make sure we are on the right lines before creating a whole implementation plan.
