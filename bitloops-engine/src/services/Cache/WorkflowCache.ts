import { IBitloopsWorkflowDefinition } from '../../entities/workflow/definitions';
import { IWorkflowCache } from '../interfaces';
import Cache from './Cache';

class WorkflowCache extends Cache<Record<string, IBitloopsWorkflowDefinition>> implements IWorkflowCache {
	constructor(max: number) {
		super(max);
	}

	async cache(workflowId: string, workflowVersion: string, environmentId: string, workflow: IBitloopsWorkflowDefinition): Promise<void> {
		let workflowPerEnv = await this.fetchWorkflowPerEnv(workflowId, workflowVersion);
		if (!workflowPerEnv) workflowPerEnv = {};
		workflowPerEnv[environmentId] = workflow;
		this.set(`${workflowId}:${workflowVersion}`, workflowPerEnv);
	}

	private fetchWorkflowPerEnv(workflowId: string, workflowVersion: string): Promise<Record<string, IBitloopsWorkflowDefinition>> {
		const res = this.get(`${workflowId}:${workflowVersion}`);
		return Promise.resolve(res);
	}

	fetch(workflowId: string, workflowVersion: string, environmentId: string): Promise<IBitloopsWorkflowDefinition | null> {
		console.log(`fetching worfklow with id: ${workflowId},  version ${workflowVersion} and environment ${environmentId}`);
		const res = this.get(`${workflowId}:${workflowVersion}`);
		if (!res) return Promise.resolve(null);
		return Promise.resolve(res[environmentId]);
	}

	delete(workflowId: string, workflowVersion: string) {
		console.log(`deleting worfklow with id: ${workflowId} from cache version ${workflowVersion}`);
		this.remove(`${workflowId}:${workflowVersion}`);
	}
}

export default WorkflowCache;
