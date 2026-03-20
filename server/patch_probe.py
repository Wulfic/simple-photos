import re

def update_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()
    
    old_code = '''    let probe_hosts: Vec<&str> = vec!["127.0.0.1", "host.docker.internal", "172.17.0.1"];

    let mut local_futures = Vec::new();'''
    
    new_code = '''    let mut probe_hosts: Vec<String> = vec![
        "127.0.0.1".to_string(), 
        "host.docker.internal".to_string(), 
        "172.17.0.1".to_string()
    ];
    if let Some(gw) = crate::backup::broadcast::get_default_gateway() {
        if !probe_hosts.contains(&gw) {
            probe_hosts.push(gw);
        }
    }

    let mut local_futures = Vec::new();'''
    
    content = content.replace(old_code, new_code)
    
    with open(filepath, 'w') as f:
        f.write(content)

update_file('server/src/setup/handlers.rs')
update_file('server/src/backup/handlers.rs')
