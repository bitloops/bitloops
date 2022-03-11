import { ISecretCache } from '../interfaces';
import Cache from './LRUCache';

class SecretCache extends Cache<string> implements ISecretCache {
	constructor(max: number) {
		super(max);
	}
	fetch(workflowId: string, workflowVersion: string) {
		return this.get(`${workflowId}:${workflowVersion}`);
	}
	cache(workflowId: string, workflowVersion: string, secrets: any) {
		this.set(`${workflowId}:${workflowVersion}`, secrets);
	}
}
export default SecretCache;
