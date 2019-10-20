from typing import List, Dict, Any

from pydgraph import DgraphClient

print('import boto3')
import boto3

print('import pydgraph')
import pydgraph

print('import json')
import json

print('import time')
import time

from hashlib import sha256


def list_all_lenses(prefix: str) -> List[Dict[str, Any]]:
    client_stub = pydgraph.DgraphClientStub('alpha0.engagementgraphcluster.grapl:9080')
    dg_client = pydgraph.DgraphClient(client_stub)

    # DGraph query for all nodes with a 'lens' that matches the 'prefix'
    if prefix:
        query = """
            query q0($a: string)
            {
                q0(func: alloftext(lens, $a), orderdesc: score)
                {
                    uid,
                    node_key,
                    lens,
                    score
                }
            }"""

        variables = {'$a': prefix}
    else:
        query = """
            {
                q0(func: has(lens), orderdesc: score)
                {
                    uid,
                    node_key,
                    lens,
                    score
                }
            }"""

        variables = {}

    txn = dg_client.txn(read_only=True)

    try:
        res = json.loads(txn.query(query, variables=variables).json)
        return res['q0']
    finally:
        txn.discard()

# Just query the schema in the future
process_properties = [
    'process_id', 'node_key', 'create_time', 'arguments',
    'process_name'
]

file_properties = [
    'node_key', 'file_path'
]


edge_names = [
    'children',
    'bin_file',
    'created_file',
    'scope',
]

# Get all nodes in a lens scope, and all of the edges from nodes in the scope to other nodes in the scope
def get_lens_scope(dg_client, lens):
    query = """
        query q0($a: string)
        {  
            q0(func: eq(lens, $a)) {
                uid,
                node_key,
                lens,
                score,
                scope {
                    uid,
                    expand(_forward_) {
                        uid,    
                        node_key,
                        process_name,
                        process_id,
                        file_path,
                        node_type,
                        port,
                        created_timestamp,
                        analyzer_name,
                        risk_score,
                        ~scope @filter(eq(lens, $a) OR has(risk_score)) {
                            uid, node_key, analyzer_name, risk_score,
                            lens, score
                        }
                    }
                }
            }  
      }"""

    txn = dg_client.txn(read_only=True)

    try:
        variables = {'$a': lens}
        res = json.loads(txn.query(query, variables=variables).json)
        return res['q0']
    finally:
        txn.discard()


def hash_node(node):
    hash_str = str(node['uid'])
    print(node)
    props = []
    for prop_name, prop_value in node:
        if isinstance(prop_value, list):
            if len(prop_value) > 0 and isinstance(prop_value[0], dict):
                if prop_value[0].get('uid'):
                    continue

        props.append(prop_name + str(prop_value))

    props.sort()
    hash_str += "".join(props)

    edges = []

    for prop_name, prop_value in node:
        if isinstance(prop_value, list):
            if len(prop_value) > 0 and isinstance(prop_value[0], dict):
                if not prop_value[0].get('uid'):
                    continue
                edge_uids = []
                for edge in prop_value:
                    edges.append(prop_name + edge['uid'])

                edge_uids.sort()
                edges.append("".join(edge_uids))

    edges.sort()
    print(edges)
    hash_str += "".join(edges)
    # return hash_str
    return sha256(hash_str.encode()).hexdigest()


def strip_graph(graph, lens, edgename='scope'):
    for outer_node in graph.get(edgename, []):
        for prop, val in outer_node.items():
            if prop == 'risks' or prop == '~risks':
                continue

            if isinstance(val, list) and isinstance(val[0], dict):
                new_vals = []
                for inner_val in val:
                    rev_scope = inner_val.get('~scope', [])
                    to_keep = False
                    for n in rev_scope:
                        if (n.get('lens') == lens) or n.get('analyzer_name'):
                            to_keep = True
                    if to_keep:
                        new_vals.append(inner_val)
                outer_node[prop] = new_vals


def get_updated_graph(dg_client, initial_graph, lens):
    current_graph = get_lens_scope(dg_client, lens)
    for graph in current_graph:
        strip_graph(graph, lens)

    new_or_modified = []
    for node in current_graph:
        if initial_graph.get(node['uid']):
            node_hash = initial_graph[node['uid']]
            if node_hash != hash_node(node):
                new_or_modified.append(node)
        else:
            new_or_modified.append(node)

    all_uids = []
    for node in current_graph:
        if node.get('scope'):
            all_uids.extend([node['uid'] for node in node.get('scope')])
        all_uids.append(node['uid'])

    removed_uids = set(initial_graph.keys()) - \
                   set(all_uids)

    return new_or_modified, list(removed_uids)


def try_get_updated_graph(body):
    print('Trying to update graph')
    client_stub = pydgraph.DgraphClientStub('alpha0.engagementgraphcluster.grapl:9080')
    dg_client = pydgraph.DgraphClient(client_stub)

    lens = body["lens"]

    # Mapping from `uid` to node hash
    initial_graph = body["uid_hashes"]

    print(f'lens: {lens} initial_graph: {initial_graph}')

    # Try for 20 seconds max
    max_time = int(time.time()) + 20
    while True:
        print("Getting updated graph")
        updated_nodes, removed_nodes = get_updated_graph(
            dg_client,
            initial_graph,
            lens
        )

        updates = {
            'updated_nodes': updated_nodes,
            'removed_nodes': removed_nodes
        }

        if updated_nodes or removed_nodes:
            print("Graph has been updated: ")
            return updates

        now = int(time.time())

        if now >= max_time:
            print("Timed out before finding an update")
            return updates
        print("Graph has not updated")
        time.sleep(0.75)


def respond(err, res=None):
    return {
        'statusCode': '400' if err else '200',
        'body': err if err else json.dumps(res),
        'headers': {
            'Access-Control-Allow-Origin': '*',
            'Content-Type': 'application/json',
            'Access-Control-Allow-Methods': 'GET,POST,OPTIONS',
            'X-Requested-With': '*',
        },
    }


def lambda_handler(event, context):
    try:
        print(f"httpMethod: {event['httpMethod']}")
        print(f"path: {event['path']}")

        if event['httpMethod'] == 'OPTIONS':
            return respond(None, {})

        if '/update' in event['path']:
            update = try_get_updated_graph(json.loads(event["body"]))
            return respond(None, update)

        if '/getLenses' in event['path']:
            prefix = json.loads(event["body"]).get('prefix', '')
            lenses = list_all_lenses(prefix)
            return respond(None, {'lenses': lenses})

        return respond(f"Invalid path: {event['path']}", {})
    except Exception as e:
        print('Failed with e {}'.format(e))
        return respond("Error fetching updates {}".format(e))

