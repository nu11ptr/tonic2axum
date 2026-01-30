# Streaming Example

Generates a REST API for a streaming gRPC Greeter service

## Example Output

### REST

```bash
% curl --json $'{"first_name": "Jane", "last_name": "Doe"}\n{"first_name": "John", "last_name": "Doe"}' 'http://127.0.0.1:8000/v1/hello'
{"message":"Hello, Jane Doe!"}
{"message":"Hello, John Doe!"}
```

### gRPC

```bash
% grpcurl -plaintext -import-path ./proto -proto ./proto/hello/v1/hello.proto -d $'{"first_name": "Jane", "last_name": "Doe"}\n{"first_name": "John", "last_name": "Doe"}' 127.0.0.1:8000 hello.v1.Greeter/SayHello
{
  "message": "Hello, Jane Doe!"
}
{
  "message": "Hello, John Doe!"
}
```