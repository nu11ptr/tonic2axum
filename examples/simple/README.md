# Simple Example

Generates a REST API for a simple gRPC Greeter service

## Example Output

### REST

```bash
% curl --json '{"last_name": "Doe"}' 'http://127.0.0.1:8000/v1/hello/John?salutation=Mr.' 
{"message":"Hello, Mr. John Doe!"}
```

### gRPC

```bash
% grpcurl -plaintext -import-path ./proto -proto ./proto/hello/v1/hello.proto -d '{"salutation": "Mr.", "first_name": "John", "last_name": "Doe"}' 127.0.0.1:8000 hello.v1.Greeter/SayHello
{
  "message": "Hello, Mr. John Doe!"
}
```