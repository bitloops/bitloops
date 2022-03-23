import bitloops from './bitloops';

(async () => {
  const [response, error] = 
    await bitloops.helloWorld.sayHello({name: "Bitloops"});
  if (!error) console.log(response);
})();