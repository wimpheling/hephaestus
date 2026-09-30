# Volume providers

The volume providers split persistent disk effects from durable lease state.
[`local/`](local/) owns single-host raw backing files and formats them as
ext4. [`postgres/`](postgres/) stores volume rows, host ownership, lease
expiry, attachment state, and monotonic fencing tokens.

The run workflow resolves an instance-state volume, asks PostgreSQL for an
exclusive lease, lets the local adapter prepare and attach the backing file,
heartbeats while the VM runs, and releases it after detachment. Recovery first
fences an expired lease in PostgreSQL and only then allows cleanup. Callers get
a `VolumeAttachment` or a typed conflict, stale-token, wrong-host, or backing
failure; a delayed worker cannot silently become the next writer.
