import { WorkflowParams } from '../workflow/definitions';
import { BaseNode } from './BaseNode';
import { ITimerIntermediateNodeType, ITimerNode, ITimerNodeType, NodeTypeName } from './definitions';

class TimerNode extends BaseNode {

	async execute(workflowParams: WorkflowParams): Promise<WorkflowParams> {
		this.startedAt = Date.now();
		const nodeDefinition = this.nodeDefinition as ITimerNode;

		await this.executeTimerNode(nodeDefinition.type);

		return super.execute(workflowParams);
	}

	private async executeTimerNode(type: ITimerNodeType) {
		switch (type.name) {
			case NodeTypeName.TimerIntermediateNode: {
				const timerNodeType = type as ITimerIntermediateNodeType;
				await this.sleep(timerNodeType.parameters.timerDuration);
				break;
			}
			default:
				console.error(`Timer node type ${type.name} - not implemented`);
				break;
		}
	}

	private async sleep(ms) {
		return new Promise(resolve => setTimeout(resolve, ms));
	}
}

export default TimerNode;
