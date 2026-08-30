using System.Linq;
using MyApp.Domain;

namespace MyApp.Infrastructure;

public class UserRepository : IRepository<User>
{
    private readonly List<User> _users = new();

    public Task<User?> FindByIdAsync(int id)
    {
        var match = _users.FirstOrDefault(u => u.Id == id);
        return Task.FromResult(match);
    }

    public Task SaveAsync(User entity)
    {
        _users.Add(entity);
        return Task.CompletedTask;
    }
}