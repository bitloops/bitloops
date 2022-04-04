import { IBitloopsWorkflowDefinition } from '../../entities/workflow/definitions';
import { IWorkflowCache } from '../interfaces';
import Cache from './Cache';

class WorkflowCache extends Cache<IBitloopsWorkflowDefinition> implements IWorkflowCache {
	constructor(max: number) {
		super(max);
	}

	cache(workflowId: string, workflowVersion: string, environmentId: string, workflow: IBitloopsWorkflowDefinition) {
		console.log(`adding worfklow with id: ${workflowId} to cache version ${workflowVersion} and environment ${environmentId}`);
		this.set(`${workflowId}:${workflowVersion}:${environmentId}`, workflow);
	}

	fetch(workflowId: string, workflowVersion: string, environmentId: string): Promise<IBitloopsWorkflowDefinition> {
		console.log(`fetching worfklow with id: ${workflowId},  version ${workflowVersion} and environment ${environmentId}`);
		const res = this.get(`${workflowId}:${workflowVersion}:${environmentId}`);
		return Promise.resolve(res);
	}

	delete(workflowId: string, workflowVersion: string, environmentId: string) {
		console.log(`deleting worfklow with id: ${workflowId} from cache version ${workflowVersion} and environment ${environmentId}`);
		this.remove(`${workflowId}:${workflowVersion}:${environmentId}`);
	}
}

export default WorkflowCache;
