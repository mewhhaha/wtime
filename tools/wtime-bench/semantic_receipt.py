"""Source-bound semantic receipt preflight, independent of paired analysis."""
from pathlib import Path
import json
import sys

def need(condition, message):
    if not condition:
        raise ValueError(message)

def validate(path, declaration, current_sources, root):
    """Bind the candidate to real named tests in a successful full verify receipt."""
    sys.path.insert(0,str(Path(root)/'tools'))
    from verification.artifacts import ArtifactStore, digest
    from verification.runner import test_results
    receipt = json.loads(Path(path).read_text())
    need(receipt.get('schema') == 'wtime-verification-phase-v1'
         and receipt.get('task') == declaration['task'] and receipt.get('phase') == 'verify'
         and receipt.get('status') == 'passed' and receipt.get('lane','full') == 'full'
         and receipt.get('synthetic') is False,
         'candidate requires a successful full semantic verification receipt')
    inputs = receipt['inputs']
    need(digest(inputs) == receipt['input_key'], 'semantic receipt inputs changed')
    recorded = {}
    for component in inputs['components'].values():
        recorded.update(component['files'])
    core = {name:value for name,value in current_sources.items()
            if name.startswith('crates/') or name in ('Cargo.toml','Cargo.lock','rust-toolchain.toml')}
    need(core and all(recorded.get(name) == value for name,value in core.items()),
         'semantic tests ran against different or incompletely bound candidate sources')
    store = ArtifactStore(Path(root)/'target/verification')
    for reference in receipt['artifacts'].values():
        store.verify(reference)
    commands = {row['name']:row for row in receipt['commands']}
    names = set()
    for group in inputs['definition'].get('tests',[]):
        command = commands[group['command']]
        need(command['exit_code'] == 0, 'semantic test command failed')
        output = store.read(command['stdout']).decode()+'\n'+store.read(command['stderr']).decode()
        result = test_results(output,group)
        need(result == receipt['tests'][group['command']], 'semantic test receipt differs from actual test logs')
        names.update(result['names'])
        names.update(name.rsplit('::',1)[-1].rsplit('.',1)[-1] for name in result['names'])
    need(set(declaration['semantic_cases']) <= names, 'required semantic cases did not execute')
    return {'status':'passed','required_cases':declaration['semantic_cases']}

