import { IWorkflow } from '../../services/interfaces';
import { WorkflowParams } from '../workflow/definitions';
import { BaseNode } from './BaseNode';
import { IStartNode, INode } from './definitions';
import { v4 } from 'uuid';
import { replaceVars } from '../../helpers/replaceVariables';

class StartNode extends BaseNode {
	private payload: Record<string, unknown>;

	public constructor(id: string, workflow: IWorkflow, nodeDefinition: INode, payload: Record<string, unknown>) {
		super(id, workflow, nodeDefinition);
		this.payload = payload;
	}

	async execute(workflowParams: WorkflowParams): Promise<WorkflowParams> {
		this.startedAt = Date.now();
		// const { id: workflowId, version } = this.workflow.getWorkflow(); TODO check if this is needed

		workflowParams.payload = this.payload;
		// TODO feature (later) check input variables to verify correct type and presence of required variables
		// and reply with rejection if not as expected
		const nodeDefinition = this.nodeDefinition as IStartNode;
		const inputsArray = nodeDefinition.type.input ?? [];

		workflowParams.variables = await replaceVars(inputsArray, workflowParams)
		return super.execute(workflowParams);
	}
}

export default StartNode;
