# Non-qualifying release-gap stress attempt

This attempt used a separate `ReceiveOnly` sink and the release-gap stress
scenario. The production session remained paused and did not join within the
bounded timeout. The report is `INCONCLUSIVE`; it collected zero release-gap
samples and is not included in the anomaly rate or the qualification result.

The ready record and complete event log are retained with the report to make
the attempt auditable. Its 25 received input events do not constitute a
complete stress run and are not used to assess release-gap behavior.
