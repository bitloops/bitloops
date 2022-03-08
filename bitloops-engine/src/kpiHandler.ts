import { Subscription, NatsConnection, JSONCodec } from 'nats';
import { NodeKPIData } from './entities/nodes/definitions';

export const kpiHandler = async (logger: Subscription, nc: NatsConnection, services): Promise<void> => {
	const jc = JSONCodec<NodeKPIData>();
	const { bigQuery } = services;
	console.info(`listening for ${logger.getSubject()} requests...`);
	for await (const m of logger) {
		const data: NodeKPIData = jc.decode(m.data);
		// // console.log(`Logging ${data.instanceId} to logging db`);
		const rows = [
			{
				client_id: data.workspaceId,
				workflow_id: data.workflowId,
				instance_id: data.instanceId,
				kpi: data.kpi,
				value: data.value,
				debug_id: data.debugId,
				occurred_at: data.occurredAt / 1000,
			},
		];
		// Insert data into the table
		await bigQuery
			.dataset('bitloops_managed_analytics')
			.table('workflow_kpis')
			.insert(rows)
			.then(() => {
				// // console.log(
				// 	`Wrote logging data for for ${data.workflowId} ${data.instanceId} ${data.kpi} ${data.debugId}`,
				// );
			})
			.catch((err) => {
				// console.log(
				// 	`Failed to write rows to bitloops_managed_analytics.workflow_kpis for ${data.workflowId} ${data.instanceId} ${data.kpi} ${data.debugId}`,
				// );
				// // console.log(rows);
				// console.error(err);
			});
	}
};
