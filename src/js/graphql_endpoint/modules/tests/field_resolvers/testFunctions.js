const fetch = require("node-fetch");
const { getDgraphClient } = require ('../../dgraph_client');
const dgraph = require("dgraph-js");
const grpc = require("grpc");


const createNode = async (node_key, dgraph_type, properties) => {
    const dgraphClient = getDgraphClient();
    const query = `
    query {
        node as var(func: eq(node_key, "${node_key}"))
    }`;

    let nquads = `
        uid(node) <node_key> "${node_key}" .
        uid(node) <dgraph.type> "${dgraph_type}" .
    `;

    for (const [prop_name, prop_value] of properties) {
        nquads += `uid(node) <${prop_name}> "${prop_value}" .
        `
    }
    const mu = new dgraph.Mutation();
    mu.setSetNquads(nquads);

    const req = new dgraph.Request();
    req.setQuery(query);
    req.setMutationsList([mu]);
    req.setCommitNow(true);
    await dgraphClient.newTxn().doRequest(req);

}

const initializeGraph = async () => {

    createNode("my_node_key", "Process", [
        ["process_id", "1234"],
        ["process_name", "chrome.exe"],
    ])

    createNode("my_node_key_0", "File", [
        ["file_path", "/home/andrea/tests/file.txt"],
    ])
}


// const testConnection = async () => {
//     let response = await fetch(
//         "http://localhost:5000/graphql",
//         {
//             credentials: 'none',
//         }
//     );

//     if(response.status === 401){
//         return true;  // checks if running but unauthorized (this is okay);
//     } else { return false }
// }


const getProcess = async (queryArgs, propertiesToFetch) => {
    let args = '';
    if (queryArgs) {
        args = `(${queryArgs})`;
    } 
    const query = `
    {
        process${args} {
            ... on Process {
                ${propertiesToFetch}    
            }
        }    
    }
    `;

    return (await fetchGraphQl(query));
}

const fetchGraphQl = async (query) => {
    // const loginRes = await login('grapluser', 'graplpassword');

    const res = await fetch(`http://localhost:5000/graphql`,
        {
            method: 'post',
            body: JSON.stringify({ query }),
            headers: {
                'Content-Type': 'application/json',
            },
            credentials: 'include',
        })
        .then(res => {
            // console.log(res);
            return res
        })
        .then(res => res.json())
        .then(res => {
            console.log('retrieveGraph res', res);
            return res
        })
        .then((res) => res.data);
    return await res;
}


// query each type

module.exports = {
    getProcess,
    initializeGraph,
}