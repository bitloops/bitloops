// const start = async () => {
// 	try {
// 		const services = await Services.initializeServices();
// 		console.log(`PORT: ${process.env.PORT || 8080}`);
// 		await server.listen(process.env.PORT || 8080, '0.0.0.0');
// 		const address = server.server.address();
// 		const family = typeof address === 'string' ? address : address?.family;
// 		const port = typeof address === 'string' ? address : address?.port;
// 		console.log(
// 			`${family} server ${typeof address === 'string' ? address : address?.address} started on port ${port}...`,
// 		);
// 	} catch (err) {
// 		server.log.error(err);
// 		console.error(err);
// 		console.log('Exiting process...');
// 		process.exit(1);
// 	}
// };
// start();
