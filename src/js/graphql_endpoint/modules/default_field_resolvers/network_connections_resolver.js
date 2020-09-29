const {  
    GraphQLInt, 
    GraphQLString, 
    GraphQLList,
}  = require('graphql');

const { getDgraphClient } = require('../dgraph_client.js');
const { getEdge, getEdges, expandTo } = require('../API/queries/edge.js');
const { NetworkConnection } = require('../node_types/process_inbound_connections.js')


const networkConnectionArgs = () => {
    return {
        src_ip_address: {type: GraphQLString}, 
        src_port: {type: GraphQLString}, 
        dst_ip_address: {type: GraphQLString}, 
        dst_port: {type: GraphQLString}, 
        created_timestamp: {type: GraphQLInt}, 
        terminated_timestamp: {type: GraphQLInt},
        last_seen_timestamp: {type: GraphQLInt},
    }
}

const networkConnectionFilters = (args) => {
    return [
        ['src_ip_address', args.src_ip_address, 'string'],
        ['src_port', args.src_port, 'string'],
        ['dst_ip_address', args.dst_ip_address, 'string'],
        ['dst_port', args.dst_port, 'string'],
        ['created_timestamp', args.created_timestamp, 'int'],
        ['terminated_timestamp', args.terminated_timestamp , 'int'],
        ['last_seen_timestamp', args.last_seen_timestamp , 'int']
    ]
}


const defaultNetworkConnectionResolver = (edgeName) => {
    return {
        type: NetworkConnection,
        args: networkConnectionArgs(),
        resolve: async(parent, args) => {
            console.log("expanding defaultNetworkConnectionResolver");
            const expanded = await expandTo(getDgraphClient(), parent.uid, edgeName, networkConnectionFilters(args), getEdge);
            console.log ("expanded networkConnection", expanded);
            return expanded;
        }
    };
};

const defaultNetworkConnectionsResolver = (edgeName) => {
    const { NetworkConnection } = require('../node_types/network_connection.js');
    return {
        type: GraphQLList(NetworkConnection),
        args: networkConnectionArgs(),
        resolve: async(parent, args) => {
            console.log("expanding defaultNetworkConnectionResolver");
            const expanded = await expandTo(getDgraphClient(), parent.uid, edgeName, networkConnectionFilters(args), getEdges);
            console.log ("expanded networkConnection", expanded);
            return expanded
        }
    };
}

module.exports = {
    defaultNetworkConnectionResolver,
    defaultNetworkConnectionsResolver
}