// controllers.js - Controller definitions (uses services from services.js)
angular.module('myApp')

.controller('MainController', ['$scope', 'UserService', function($scope, UserService) {
    $scope.users = [];

    $scope.loadUsers = function() {
        UserService.getAll().then(function(users) {
            $scope.users = users;
        });
    };

    $scope.loadUsers();
}])

.controller('DetailController', ['$scope', '$routeParams', 'UserService', function($scope, $routeParams, UserService) {
    $scope.user = null;

    UserService.getById($routeParams.id).then(function(user) {
        $scope.user = user;
    });
}])

.controller('LoginController', ['$scope', 'AuthService', function($scope, AuthService) {
    $scope.credentials = { username: '', password: '' };

    $scope.login = function() {
        AuthService.login($scope.credentials).then(function(user) {
            $scope.currentUser = user;
        });
    };

    $scope.logout = function() {
        AuthService.logout();
    };
}]);
