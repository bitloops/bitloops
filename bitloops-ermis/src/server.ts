import Services from './services';
import { build } from './app';

let shutDownCalled = false;

const handleShutdown = async () => {
    if (shutDownCalled) return;
    console.log('Server gracefully shutting down...');
    const services = Services.getServices();
    if (services) {
        const { mq } = services;
        console.info('Quitting mq connection...');
        await mq.gracefullyCloseConnection();
    }
    shutDownCalled = true;
};

//do something when app is closing
process.on('exit', async () => {
    console.info('exit');
    await handleShutdown();
});

//catches ctrl+c event
process.on('SIGINT', async () => {
    console.info('SIGINT');
    await handleShutdown();
    process.exit(1);
});

process.on('SIGTERM', async () => {
    console.info('SIGTERM');
    await handleShutdown();
    process.exit(1);
});

const start = async () => {
    try {
        const server = await build({ logger: true, trustProxy: true });
        console.log(`PORT: ${process.env.PORT || 8080}`);
        await server.listen(process.env.PORT || 8080, '0.0.0.0');
        const address = server.server.address();
        const family = typeof address === 'string' ? address : address?.family;
        const port = typeof address === 'string' ? address : address?.port;
        console.log(
            `${family} server ${typeof address === 'string' ? address : address?.address} started on port ${port}...`,
        );
    } catch (err) {
        console.error(err);
        console.log('Exiting process...');
        process.exit(1);
    }
};
start();
