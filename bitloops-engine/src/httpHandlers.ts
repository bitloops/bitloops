import { ILRUCache } from './services/interfaces/index';
import { Options } from './services';
import { IServices } from './services/definitions';
import { memoryUsage } from 'process';

export const healthyHandler = (_, res) => {
	if (Options.getOption('needsRestart') === 'true') res.status(503).send('UNAVAILABLE');
	else res.send('OK');
};

export const readyHandler = (_, res) => {
	if (Options.getOption('mqReady') === 'true' && Options.getOption('dbReady') === 'true') res.send('OK');
	else {
		if (Options.getOption('mqReady') !== 'true') console.info('MQ is not ready...');
		if (Options.getOption('dbReady') !== 'true') console.info('DB is not ready...');
		res.status(503).send('UNAVAILABLE');
	}
};

export const cachesHandler = (services: IServices) => async (_, res) => {
	try {
		const { workflowCache, workflowEventTriggerCache, runningWorkflowInstanceCache, secretCache } = services;
		const caches = { workflowCache, workflowEventTriggerCache, secretCache };
		const itemsCount = Object.entries(caches).map(([name, cache]) => {
			const count = cache.getCount();
			return { name, count };
		});
		itemsCount.push({ name: 'runningWorkflowInstanceCache', count: await runningWorkflowInstanceCache.getCount() });
		const memoryUsed = memoryStats();

		res.send({ itemsCount, memoryUsed });
	} catch (error) {
		console.error(error);
		res.status(503).send('UNAVAILABLE');
	}

	// else res.send('OK');
};

const memoryStats = () => {
	// measured in bytes
	const used = process.memoryUsage();
	for (let key in used) {
		used[key] = `${Math.round((used[key] / 1024 / 1024) * 100) / 100} MB`;
	}
	return used;
};
