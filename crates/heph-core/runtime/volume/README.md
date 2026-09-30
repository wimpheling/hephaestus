# Volume contracts

The volume branch governs persistent state attached to reusable agent
instances. [`trait/`](trait/) describes metadata, exclusive writable leases,
attachment state, fencing generations, heartbeats, and supervised recovery.
The backing-file and database adapters implement those contracts for the local
host.

The run workflow resolves the instance-state volume before provisioning a VM.
It acquires a lease for the run, attaches the volume with its fencing token,
heartbeats while the run is live, and releases it after detachment. If a
supervisor crashes, recovery first fences the expired lease and only then lets
the provider clean up the attachment. This ordering prevents a delayed worker
from writing to a volume after another run owns it.

The volume contract is deliberately separate from workspace and result paths:
state persists with the agent instance, while source and result workspaces are
per-run materializations managed by [`../workspace/`](../workspace/).
